use crate::db::Db;
use crate::models::*;
use crate::router::AiRouter;
use crate::sse::Broadcaster;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{sse::Event, Sse},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub broadcaster: Broadcaster,
    pub router: Arc<Option<AiRouter>>,
}

impl AppState {
    pub fn db(&self) -> anyhow::Result<Db> {
        Db::open(&self.db_path)
    }
}

fn db_err<E: std::fmt::Display>(e: E) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("db error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// GET /health — liveness check
pub async fn health() -> &'static str {
    "ok"
}

/// GET /state — full dashboard init payload
pub async fn get_state(State(state): State<AppState>) -> Result<Json<DashboardState>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    let notifications = db.list_open_notifications().map_err(db_err)?;
    let history = db.list_history(50).map_err(db_err)?;
    let rules = db.list_rules().map_err(db_err)?;
    let agents_raw = db.list_agents().map_err(db_err)?;
    let mut agents = HashMap::new();
    for a in agents_raw {
        agents.insert(a.name.clone(), a);
    }
    Ok(Json(DashboardState {
        notifications,
        history,
        rules,
        agents,
    }))
}

/// POST /ask — create a new notification
pub async fn ask(
    State(state): State<AppState>,
    Json(req): Json<AskRequest>,
) -> Result<Json<Notification>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;

    // Check idempotency: if caller_id provided and exists within 24h, return existing
    if let Some(ref caller_id) = req.id {
        let since = Utc::now() - chrono::Duration::hours(24);
        if let Ok(Some(existing)) = db.get_notification_by_caller_id(caller_id, since) {
            return Ok(Json(existing));
        }
    }

    // Resolve agent identity
    let _agent = db.get_or_create_agent(&req.agent_name).map_err(db_err)?;

    // Determine question type
    let mut question_type = req.question_type.clone().unwrap_or(QuestionType::Ack);
    let mut _renderer_guessed = false;

    // If no explicit type and we have an AI router, guess it
    let router_result = if req.question_type.is_none() {
        if let Some(ref router) = *state.router {
            let open = db.list_open_notifications().unwrap_or_default();
            let result = router.classify(&req.question, &req.agent_name, req.urgency, &open).await;
            if let Some(ref suggested) = result.renderer_suggested {
                if let Ok(parsed) = suggested.parse::<QuestionType>() {
                    question_type = parsed;
                    _renderer_guessed = result.renderer_guessed;
                }
            }
            Some(result)
        } else {
            None
        }
    } else {
        None
    };

    // Parse deadline
    let deadline = req.deadline.as_ref().and_then(|d| {
        parse_deadline(d).ok()
    });

    // Parse reply_to
    let reply_to = req.reply_to.as_ref().and_then(|s| {
        if s.starts_with("webhook:") {
            Some(ReplyTo::Webhook { url: s.strip_prefix("webhook:").unwrap_or(s).to_string() })
        } else if s.starts_with("file:") {
            Some(ReplyTo::File { path: s.strip_prefix("file:").unwrap_or(s).to_string() })
        } else if s == "exit-code" {
            Some(ReplyTo::ExitCode)
        } else {
            Some(ReplyTo::Stdout)
        }
    }).unwrap_or(ReplyTo::Stdout);

    // Build sender and src
    let sender = req.instance.as_ref()
        .map(|i| format!("{} · {}", req.agent_name, i))
        .unwrap_or_else(|| req.agent_name.clone());
    let src = sender.clone();

    let id = generate_id();
    let created_at = Utc::now();

    let mut notification = Notification {
        id: id.clone(),
        agent_name: req.agent_name.clone(),
        instance: req.instance.clone(),
        sender: req.agent_name.clone(),
        src,
        question: req.question.clone(),
        detail: req.detail.clone(),
        question_type,
        urgency: req.urgency,
        blocking: req.blocking,
        deadline,
        reply_to,
        status: NotificationStatus::Open,
        created_at,
        answered_at: None,
        answer: None,
        answer_label: None,
        choices: req.choices.clone(),
        yes_label: req.yes_label.clone(),
        no_label: req.no_label.clone(),
        placeholder: req.placeholder.clone(),
        suggestions: req.suggestions.clone(),
        min: req.min,
        max: req.max,
        step: req.step,
        default_value: req.default_value,
        unit: req.unit.clone(),
        accept: req.accept.clone(),
        diff: req.diff.clone(),
        ack_label: req.ack_label.clone(),
        items: req.items.clone(),
        slots: req.slots.clone(),
        mute_key: req.mute_key.clone(),
        caller_id: req.id.clone(),
    };

    // Apply urgency from AI router if confidence is high
    if let Some(ref result) = router_result {
        if let Some(predicted) = result.urgency_predicted {
            if let Some(confidence) = result.confidence {
                if confidence > 0.7 {
                    notification.urgency = predicted;
                }
            }
        }
    }

    db.insert_notification(&notification).map_err(db_err)?;

    // Broadcast to all SSE clients
    state.broadcaster.broadcast(&SseEvent::NotificationCreated { notification: notification.clone() });

    Ok(Json(notification))
}

/// POST /answer/:id — user answers via dashboard
pub async fn answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<Json<AnswerEnvelope>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;

    let notif = match db.get_notification(&id) {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => return Err(db_err(e)),
    };

    if notif.status != NotificationStatus::Open {
        return Err((StatusCode::CONFLICT, Json(serde_json::json!({"error": "notification already answered or cancelled"}))));
    }

    let now = Utc::now();
    let latency = now.signed_duration_since(notif.created_at).num_milliseconds();

    db.answer_notification(&id, &req.answer, req.via.as_deref().or(Some(&req.answer))).map_err(db_err)?;

    let updated = db.get_notification(&id).unwrap_or(Some(notif.clone())).unwrap_or(notif.clone());

    let envelope = AnswerEnvelope {
        id: id.clone(),
        answer: Some(req.answer.clone()),
        answer_label: updated.answer_label.clone(),
        answered_at: Some(now),
        latency_ms: Some(latency),
        renderer: updated.question_type.to_string(),
        src: updated.src.clone(),
        via: req.via.unwrap_or_else(|| "dashboard".to_string()),
    };

    // Broadcast
    state.broadcaster.broadcast(&SseEvent::NotificationAnswered { envelope: envelope.clone() });

    // Deliver response back to caller
    tokio::spawn(deliver_response(updated.clone(), req.answer.clone()));

    Ok(Json(envelope))
}

/// DELETE /cancel/:id
pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    match db.get_notification(&id) {
        Ok(Some(notif)) => {
            if notif.status == NotificationStatus::Answered {
                return Ok(StatusCode::NO_CONTENT);
            }
            db.update_status(&id, NotificationStatus::Cancelled).map_err(db_err)?;
            state.broadcaster.broadcast(&SseEvent::NotificationCancelled { id: id.clone() });
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => Err(db_err(e)),
    }
}

/// GET /list — dump open notifications
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Notification>>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    let notifs = db.list_open_notifications().map_err(db_err)?;
    Ok(Json(notifs))
}

/// GET /history — answered / timed out
#[derive(Deserialize)]
pub struct HistoryQuery {
    limit: Option<usize>,
}
pub async fn history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<Notification>>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    let items = db.list_history(q.limit.unwrap_or(50)).map_err(db_err)?;
    Ok(Json(items))
}

/// GET /events — SSE stream for live dashboard updates
pub async fn events(State(state): State<AppState>) -> Sse<impl tokio_stream::Stream<Item = Result<Event, axum::Error>>> {
    let rx = state.broadcaster.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(json) => Some(Ok(Event::default().data(json))),
            Err(_) => None, // lagged, drop
        }
    });
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text(""),
    )
}

/// POST /rules — create a rule
pub async fn create_rule(
    State(state): State<AppState>,
    Json(body): Json<CreateRuleBody>,
) -> Result<Json<Rule>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;

    let rule = Rule {
        id: format!("r-{}", nanoid::nanoid!(6)),
        text: body.text.clone(),
        compiled: body.compiled.clone(),
        active: true,
        scope: body.scope.clone(),
        urgency_min: body.urgency_min.unwrap_or(0),
        mute: body.mute.unwrap_or(false),
        expires_at: None,
        active_window: None,
        created_at: Utc::now(),
    };

    db.insert_rule(&rule).map_err(db_err)?;

    state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: rule.clone() });
    Ok(Json(rule))
}

/// DELETE /rules/:id
pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    db.delete_rule(&id).map_err(db_err)?;
    state.broadcaster.broadcast(&SseEvent::RuleDeleted { id });
    Ok(StatusCode::NO_CONTENT)
}

/// GET /agents
pub async fn list_agents(State(state): State<AppState>) -> Result<Json<HashMap<String, Agent>>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    let raw = db.list_agents().map_err(db_err)?;
    let mut agents = HashMap::new();
    for a in raw {
        agents.insert(a.name.clone(), a);
    }
    Ok(Json(agents))
}

/// POST /agents — register / override an agent identity
pub async fn register_agent(
    State(state): State<AppState>,
    Json(agent): Json<Agent>,
) -> Result<Json<Agent>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db().map_err(db_err)?;
    db.upsert_agent(&agent).map_err(db_err)?;
    Ok(Json(agent))
}

/// Background task: deliver response back to caller
async fn deliver_response(_notif: Notification, _answer: String) {
    // TODO: implement webhook POST, file write, exit-code signal
    // For now, the blocking/long-poll path is handled via SSE + the /answer/:id response
    tracing::info!("would deliver answer for {} via {:?}", _notif.id, _notif.reply_to);
}

#[derive(Deserialize)]
pub struct CreateRuleBody {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgency_min: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
}
