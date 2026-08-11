use crate::handlers::*;
use crate::models::*;
use crate::db::Db;
use crate::router::AiRouter;
use crate::sse::Broadcaster;
use crate::push::ApnsClient;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

pub async fn run_daemon(
    port: u16,
    api_key: Option<String>,
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

    let app = Router::new()
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
        .fallback_service(ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    tracing::info!("sjbis {} daemon listening on http://{}", crate::version::full(), addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
