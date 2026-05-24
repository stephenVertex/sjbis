use crate::db::Db;
use crate::models::*;
use crate::router::AiRouter;
use crate::rules;
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
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// In-memory waiters for blocking callers
pub type Waiters = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<AnswerEnvelope>>>>;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub broadcaster: Broadcaster,
    pub router: Arc<Option<AiRouter>>,
    pub waiters: Waiters,
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
    let notifications = state.db.list_open_notifications().await.map_err(db_err)?;
    let history = state.db.list_history(50).await.map_err(db_err)?;
    let rules = state.db.list_rules().await.map_err(db_err)?;
    let agents_raw = state.db.list_agents().await.map_err(db_err)?;
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
    // Check idempotency: if caller_id provided and exists within 24h, return existing
    if let Some(ref caller_id) = req.id {
        let since = Utc::now() - chrono::Duration::hours(24);
        if let Ok(Some(existing)) = state.db.get_notification_by_caller_id(caller_id, since).await {
            return Ok(Json(existing));
        }
    }

    // Resolve agent identity
    let _agent = state.db.get_or_create_agent(&req.agent_name).await.map_err(db_err)?;

    // Determine question type
    let mut question_type = req.question_type.clone().unwrap_or(QuestionType::Ack);
    let mut _renderer_guessed = false;

    // If no explicit type and we have an AI router, guess it
    let router_result = if req.question_type.is_none() {
        if let Some(ref router) = *state.router {
            let open = state.db.list_open_notifications().await.unwrap_or_default();
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
        snooze_until: None,
        note: None,
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

    // Load and evaluate rules
    let db_rules = state.db.list_rules().await.map_err(db_err)?;
    let (mut notification, auto_answer) = rules::evaluate(&db_rules, notification);

    if let Some(answer) = auto_answer {
        // Auto-answer: store and broadcast immediately
        let now = Utc::now();
        state.db.answer_notification(&notification.id, &answer, Some(&answer), None).await.map_err(db_err)?;
        notification.status = NotificationStatus::Answered;
        notification.answer = Some(answer.clone());
        notification.answered_at = Some(now);

        let envelope = AnswerEnvelope {
            id: notification.id.clone(),
            answer: Some(answer.clone()),
            answer_label: Some(answer.clone()),
            answered_at: Some(now),
            latency_ms: Some(0),
            renderer: notification.question_type.to_string(),
            src: notification.src.clone(),
            via: "rule_auto_answer".to_string(),
            note: None,
        };
        state.broadcaster.broadcast(&SseEvent::NotificationAnswered { envelope });
        return Ok(Json(notification));
    }

    if notification.status == NotificationStatus::Muted {
        state.db.insert_notification(&notification).await.map_err(db_err)?;
        // Don't broadcast muted notifications
        return Ok(Json(notification));
    }

    state.db.insert_notification(&notification).await.map_err(db_err)?;

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
    let notif = match state.db.get_notification(&id).await {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => return Err(db_err(e)),
    };

    if notif.status != NotificationStatus::Open {
        return Err((StatusCode::CONFLICT, Json(serde_json::json!({"error": "notification already answered or cancelled"}))));
    }

    let now = Utc::now();
    let latency = now.signed_duration_since(notif.created_at).num_milliseconds();

    state.db.answer_notification(&id, &req.answer, req.via.as_deref().or(Some(&req.answer)), req.note.as_deref()).await.map_err(db_err)?;

    let updated = state.db.get_notification(&id).await.unwrap_or(Some(notif.clone())).unwrap_or(notif.clone());

    let envelope = AnswerEnvelope {
        id: id.clone(),
        answer: Some(req.answer.clone()),
        answer_label: updated.answer_label.clone(),
        answered_at: Some(now),
        latency_ms: Some(latency),
        renderer: updated.question_type.to_string(),
        src: updated.src.clone(),
        via: req.via.unwrap_or_else(|| "dashboard".to_string()),
        note: updated.note.clone(),
    };

    // Notify any blocking waiters
    {
        let mut waiters = state.waiters.lock().await;
        if let Some(tx) = waiters.remove(&id) {
            let _ = tx.send(envelope.clone());
        }
    }

    // Broadcast
    state.broadcaster.broadcast(&SseEvent::NotificationAnswered { envelope: envelope.clone() });

    // Deliver response back to caller (webhook, file, etc.)
    tokio::spawn(deliver_response(updated.clone(), req.answer.clone(), envelope.clone()));

    Ok(Json(envelope))
}

/// DELETE /cancel/:id
pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    match state.db.get_notification(&id).await {
        Ok(Some(notif)) => {
            if notif.status == NotificationStatus::Answered {
                return Ok(StatusCode::NO_CONTENT);
            }
            state.db.update_status(&id, NotificationStatus::Cancelled).await.map_err(db_err)?;
            state.broadcaster.broadcast(&SseEvent::NotificationCancelled { id: id.clone() });
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => Err(db_err(e)),
    }
}

/// POST /snooze/:id — push notification back by N minutes (capped at deadline)
#[derive(Deserialize)]
pub struct SnoozeBody {
    pub minutes: i64,
}
pub async fn snooze(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SnoozeBody>,
) -> Result<Json<Notification>, (StatusCode, Json<serde_json::Value>)> {
    // Fetch the notification first to validate
    let notif = match state.db.get_notification(&id).await {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => return Err(db_err(e)),
    };

    if notif.status != NotificationStatus::Open {
        return Err((StatusCode::CONFLICT, Json(serde_json::json!({"error": "notification is not open"}))));
    }

    // Validate: snooze cannot extend past the auto-approve deadline
    let now = Utc::now();
    let proposed = now + chrono::Duration::minutes(body.minutes);
    if let Some(deadline) = notif.deadline {
        if proposed > deadline {
            let remaining = (deadline - now).num_seconds();
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": "snooze exceeds auto-approve deadline",
                "remaining_seconds": remaining.max(0),
                "deadline": deadline,
            }))));
        }
    }

    let updated = state.db.snooze_notification(&id, body.minutes).await.map_err(db_err)?;
    match updated {
        Some(n) => {
            state.broadcaster.broadcast(&SseEvent::NotificationUpdated { notification: n.clone() });
            Ok(Json(n))
        }
        None => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
    }
}

/// GET /list — dump open notifications
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Notification>>, (StatusCode, Json<serde_json::Value>)> {
    let notifs = state.db.list_open_notifications().await.map_err(db_err)?;
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
    let items = state.db.list_history(q.limit.unwrap_or(50)).await.map_err(db_err)?;
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

    state.db.insert_rule(&rule).await.map_err(db_err)?;

    state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: rule.clone() });
    Ok(Json(rule))
}

/// DELETE /rules/:id
pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    state.db.delete_rule(&id).await.map_err(db_err)?;
    state.broadcaster.broadcast(&SseEvent::RuleDeleted { id });
    Ok(StatusCode::NO_CONTENT)
}

/// GET /agents
pub async fn list_agents(State(state): State<AppState>) -> Result<Json<HashMap<String, Agent>>, (StatusCode, Json<serde_json::Value>)> {
    let raw = state.db.list_agents().await.map_err(db_err)?;
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
    state.db.upsert_agent(&agent).await.map_err(db_err)?;
    Ok(Json(agent))
}

/// GET /wait/{id} — blocking wait for an answer
pub async fn wait_for_answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AnswerEnvelope>, (StatusCode, Json<serde_json::Value>)> {
    // First check if already answered
    if let Ok(Some(notif)) = state.db.get_notification(&id).await {
        if notif.status == NotificationStatus::Answered {
            let envelope = AnswerEnvelope {
                id: id.clone(),
                answer: notif.answer.clone(),
                answer_label: notif.answer_label.clone(),
                answered_at: notif.answered_at,
                latency_ms: notif.answered_at.map(|at| at.signed_duration_since(notif.created_at).num_milliseconds()),
                renderer: notif.question_type.to_string(),
                src: notif.src.clone(),
                via: "dashboard".to_string(),
                note: notif.note.clone(),
            };
            return Ok(Json(envelope));
        }
    }

    // Set up a oneshot waiter
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut waiters = state.waiters.lock().await;
        waiters.insert(id.clone(), tx);
    }

    // Wait with a timeout (default 5 minutes)
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(envelope)) => Ok(Json(envelope)),
        Ok(Err(_)) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "waiter cancelled"})))),
        Err(_) => {
            // Timeout: clean up waiter
            let mut waiters = state.waiters.lock().await;
            waiters.remove(&id);
            Err((StatusCode::REQUEST_TIMEOUT, Json(serde_json::json!({"error": "timeout waiting for answer"}))))
        }
    }
}

/// Background task: deliver response back to caller
async fn deliver_response(notif: Notification, _answer: String, envelope: AnswerEnvelope) {
    match notif.reply_to {
        ReplyTo::Webhook { url } => {
            let client = reqwest::Client::new();
            match client.post(&url).json(&envelope).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!("webhook delivered to {} for {}", url, notif.id);
                }
                Ok(resp) => {
                    tracing::warn!("webhook failed for {}: status {}", notif.id, resp.status());
                }
                Err(e) => {
                    tracing::warn!("webhook error for {}: {}", notif.id, e);
                }
            }
        }
        ReplyTo::File { path } => {
            match tokio::fs::write(&path, serde_json::to_string_pretty(&envelope).unwrap_or_default()).await {
                Ok(_) => tracing::info!("file written to {} for {}", path, notif.id),
                Err(e) => tracing::warn!("file write error for {}: {}", notif.id, e),
            }
        }
        ReplyTo::Stdout | ReplyTo::ExitCode => {
            // Blocking callers use the /wait/{id} endpoint or CLI long-poll
            tracing::info!("answer for {} ready for blocking caller", notif.id);
        }
    }
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
