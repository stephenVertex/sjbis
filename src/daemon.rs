use crate::handlers::*;
use crate::models::*;
use crate::db::Db;
use crate::router::AiRouter;
use crate::sse::Broadcaster;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

pub async fn run_daemon(
    db_path: PathBuf,
    port: u16,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    // Seed default agents if db doesn't exist or is empty
    if let Ok(db) = Db::open(&db_path) {
        let existing = db.list_agents()?;
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
                let _ = db.upsert_agent(&a);
            }
        }
    }

    let router = api_key.map(AiRouter::new);

    let state = AppState {
        db_path,
        broadcaster: Broadcaster::new(),
        router: Arc::new(router),
        waiters: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/state", get(get_state))
        .route("/ask", post(ask))
        .route("/answer/{id}", post(answer))
        .route("/cancel/{id}", delete(cancel))
        .route("/list", get(list))
        .route("/history", get(history))
        .route("/events", get(events))
        .route("/wait/{id}", get(wait_for_answer))
        .route("/rules", post(create_rule))
        .route("/rules/{id}", delete(delete_rule))
        .route("/agents", get(list_agents).post(register_agent))
        .fallback_service(ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    tracing::info!("sjbis daemon listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
