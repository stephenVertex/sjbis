use crate::handlers::*;
use crate::models::*;
use crate::db::Db;
use crate::router::AiRouter;
use crate::sse::Broadcaster;
use crate::push::ApnsClient;
use axum::{
    response::Redirect,
    routing::{delete, get, get_service, post},
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasePath(String);

impl BasePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }
}

impl Default for BasePath {
    fn default() -> Self {
        Self("/".to_string())
    }
}

impl std::fmt::Display for BasePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for BasePath {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("base path cannot be empty".to_string());
        }
        if value.contains(['?', '#']) {
            return Err("base path must not contain a query string or fragment".to_string());
        }
        if value.contains('\\') {
            return Err("base path must use forward slashes".to_string());
        }

        let unwrapped = value.trim_matches('/');
        if unwrapped.is_empty() {
            return Ok(Self::default());
        }

        for segment in unwrapped.split('/') {
            if segment.is_empty() {
                return Err("base path must not contain empty segments".to_string());
            }
            if segment == "." || segment == ".." {
                return Err("base path must not contain traversal segments".to_string());
            }
            if !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
            {
                return Err(format!(
                    "base path segment `{segment}` contains unsupported characters"
                ));
            }
        }

        Ok(Self(format!("/{unwrapped}")))
    }
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/state", get(get_state))
        .route("/ask", post(ask))
        .route("/answer/{id}", post(answer))
        .route("/cancel/{id}", delete(cancel))
        .route("/dismiss/{id}", post(dismiss))
        .route("/snooze/{id}", post(snooze))
        .route("/list", get(list))
        .route("/notification/{id}", get(get_notification))
        .route("/history", get(history))
        .route("/events", get(events))
        .route("/wait/{id}", get(wait_for_answer))
        .route("/rules", post(create_rule))
        .route("/rules/{id}", delete(delete_rule))
        .route("/agents", get(list_agents).post(register_agent))
        .route("/device/register", post(register_device))
        .route("/device/unregister", post(unregister_device))
}

fn public_routes<S>(static_dir: &Path) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let index = static_dir.join("index.html");
    Router::new()
        .route_service("/", get_service(ServeFile::new(index.clone())))
        .route_service("/index.html", get_service(ServeFile::new(index.clone())))
        .route_service(
            "/card/{id}",
            get_service(ServeFile::new(index)),
        )
        .fallback_service(ServeDir::new(static_dir))
}

fn build_app<S>(api: Router<S>, static_dir: &Path, base_path: &BasePath) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let app = api.merge(public_routes(static_dir));
    if base_path.is_root() {
        app
    } else {
        let dashboard_path = format!("{}/", base_path.as_str());
        let redirect_path = dashboard_path.clone();
        Router::new()
            .route(
                base_path.as_str(),
                get(move || {
                    let redirect_path = redirect_path.clone();
                    async move { Redirect::permanent(&redirect_path) }
                }),
            )
            .nest(&dashboard_path, app)
    }
}

pub async fn run_daemon(
    port: u16,
    api_key: Option<String>,
    base_path: BasePath,
) -> anyhow::Result<()> {
    let dsn = crate::cli::load_dsn()?;
    let db = Db::connect(&dsn).await?;

    // Seed default agents if empty
    let existing = db.list_agents().await?;
    if existing.is_empty() {
        let defaults = vec![
            Agent { name: "inbox-agent".to_string(), glyph: "◐".to_string(), color: agent_color("inbox-agent"), kind: "email".to_string() },
            Agent { name: "cal-agent".to_string(), glyph: "◧".to_string(), color: agent_color("cal-agent"), kind: "schedule".to_string() },
            Agent { name: "code-agent".to_string(), glyph: "⌬".to_string(), color: agent_color("code-agent"), kind: "code".to_string() },
            Agent { name: "pay-agent".to_string(), glyph: "$".to_string(), color: agent_color("pay-agent"), kind: "finance".to_string() },
            Agent { name: "fam".to_string(), glyph: "♡".to_string(), color: agent_color("fam"), kind: "people".to_string() },
            Agent { name: "shop-agent".to_string(), glyph: "☁".to_string(), color: agent_color("shop-agent"), kind: "commerce".to_string() },
            Agent { name: "doc-agent".to_string(), glyph: "¶".to_string(), color: agent_color("doc-agent"), kind: "docs".to_string() },
            Agent { name: "guard".to_string(), glyph: "⌖".to_string(), color: agent_color("guard"), kind: "security".to_string() },
            Agent { name: "tax-agent".to_string(), glyph: "∑".to_string(), color: agent_color("tax-agent"), kind: "finance".to_string() },
            Agent { name: "travel".to_string(), glyph: "✈".to_string(), color: agent_color("travel"), kind: "travel".to_string() },
        ];
        for a in defaults {
            let _ = db.upsert_agent(&a).await;
        }
    }

    let router = api_key.map(AiRouter::new);

    // Initialize APNs client if configured
    let apns = if let (Ok(key_path), Ok(team_id), Ok(key_id)) = (
        std::env::var("APNS_KEY_PATH"),
        std::env::var("APNS_TEAM_ID"),
        std::env::var("APNS_KEY_ID"),
    ) {
        match std::fs::read_to_string(&key_path) {
            Ok(pem) => {
                match ApnsClient::new(&team_id, &key_id, &pem) {
                    Ok(client) => {
                        tracing::info!("APNs push notifications enabled (team={}, key={})", team_id, key_id);
                        Some(client)
                    }
                    Err(e) => {
                        tracing::warn!("APNs client init failed: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("APNs key file not found at {}: {}", key_path, e);
                None
            }
        }
    } else {
        tracing::info!("APNs not configured (set APNS_KEY_PATH, APNS_TEAM_ID, APNS_KEY_ID to enable push)");
        None
    };

    let state = AppState {
        db,
        broadcaster: Broadcaster::new(),
        router: Arc::new(router),
        waiters: Arc::new(Mutex::new(HashMap::new())),
        apns: Arc::new(Mutex::new(apns)),
    };

    let app = build_app(api_routes(), Path::new("static"), &base_path)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    tracing::info!(
        "sjbis {} daemon listening on http://{}{}",
        crate::version::full(),
        addr,
        if base_path.is_root() { "" } else { base_path.as_str() },
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn static_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("static")
    }

    fn test_app(base_path: &str) -> Router {
        let base_path = BasePath::from_str(base_path).expect("valid test base path");
        let api = Router::new().route("/health", get(health));
        build_app(api, &static_dir(), &base_path)
    }

    async fn request(app: Router, method: Method, uri: &str) -> (StatusCode, String) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn root_routes_serve_api_assets_and_card_spa() {
        let app = test_app("/");

        let (dashboard_status, dashboard_body) = request(app.clone(), Method::GET, "/").await;
        assert_eq!(dashboard_status, StatusCode::OK);
        assert!(dashboard_body.contains("<title>SJBIS"));

        assert_eq!(
            request(app.clone(), Method::GET, "/health").await,
            (StatusCode::OK, "ok".to_string()),
        );
        let (asset_status, asset_body) = request(app.clone(), Method::GET, "/styles.css").await;
        assert_eq!(asset_status, StatusCode::OK);
        assert!(asset_body.contains(":root"));

        let (card_status, card_body) = request(app.clone(), Method::GET, "/card/sjbis-example").await;
        assert_eq!(card_status, StatusCode::OK);
        assert!(card_body.contains("<title>SJBIS"));

        let (post_status, _) = request(app, Method::POST, "/card/sjbis-example").await;
        assert_eq!(post_status, StatusCode::METHOD_NOT_ALLOWED);

        let app = test_app("/");
        let (missing_status, missing_body) = request(app, Method::GET, "/missing-api").await;
        assert_eq!(missing_status, StatusCode::NOT_FOUND);
        assert!(!missing_body.contains("<title>SJBIS"));
    }

    #[tokio::test]
    async fn prefixed_routes_keep_the_entire_app_under_the_prefix() {
        let app = test_app("/sjbis/");

        let redirect = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sjbis")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(redirect.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(redirect.headers()["location"], "/sjbis/");
        assert_eq!(
            request(app.clone(), Method::GET, "/sjbis/").await.0,
            StatusCode::OK,
        );
        assert_eq!(
            request(app.clone(), Method::GET, "/sjbis/health").await,
            (StatusCode::OK, "ok".to_string()),
        );
        assert_eq!(
            request(app.clone(), Method::GET, "/sjbis/styles.css").await.0,
            StatusCode::OK,
        );
        assert_eq!(
            request(app.clone(), Method::GET, "/sjbis/card/sjbis-example").await.0,
            StatusCode::OK,
        );
        assert_eq!(
            request(app.clone(), Method::GET, "/health").await.0,
            StatusCode::NOT_FOUND,
        );
        assert_eq!(
            request(app, Method::GET, "/card/sjbis-example").await.0,
            StatusCode::NOT_FOUND,
        );
    }

    #[test]
    fn base_paths_are_normalized() {
        assert_eq!(BasePath::from_str("/").unwrap().as_str(), "/");
        assert_eq!(BasePath::from_str("////").unwrap().as_str(), "/");
        assert_eq!(BasePath::from_str("sjbis").unwrap().as_str(), "/sjbis");
        assert_eq!(BasePath::from_str("/sjbis/").unwrap().as_str(), "/sjbis");
        assert_eq!(
            BasePath::from_str("/tools/sjbis/").unwrap().as_str(),
            "/tools/sjbis"
        );
        assert_eq!(BasePath::from_str("/v1.0/").unwrap().as_str(), "/v1.0");
    }

    #[test]
    fn invalid_base_paths_are_rejected() {
        for value in [
            "",
            "/sjbis?debug=1",
            "/sjbis#focus",
            "/../sjbis",
            "/sjbis/./app",
            "/sjbis//app",
            "/sjbis%2Fapp",
        ] {
            assert!(
                BasePath::from_str(value).is_err(),
                "expected `{value}` to be rejected"
            );
        }
    }
}
