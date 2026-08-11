use crate::db::Db;
use crate::models::*;
use crate::router::AiRouter;
use crate::rules;
use crate::sse::Broadcaster;
use crate::push::ApnsClient;
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
    pub apns: Arc<Mutex<Option<ApnsClient>>>,
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

/// GET /version — build/version info
pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": crate::version::PKG_VERSION,
        "git": crate::version::GIT_HASH,
        "full": crate::version::full(),
        "build_time": crate::version::build_time_rfc3339(),
    }))
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
        version: crate::version::full(),
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
        detail_markdown: req.detail_markdown.clone(),
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
        sub_questions: req.sub_questions.clone(),
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

    // Send push notifications to registered iOS devices
    if notification.urgency >= 2 {
        send_push_notifications(&state, &notification).await;
    }

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

/// POST /dismiss/:id — mark as dismissed without sending a reply
pub async fn dismiss(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AnswerEnvelope>, (StatusCode, Json<serde_json::Value>)> {
    let notif = match state.db.get_notification(&id).await {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
        Err(e) => return Err(db_err(e)),
    };

    if notif.status != NotificationStatus::Open {
        return Err((StatusCode::CONFLICT, Json(serde_json::json!({"error": "notification already answered, cancelled, or dismissed"}))));
    }

    let now = Utc::now();
    let latency = now.signed_duration_since(notif.created_at).num_milliseconds();

    state.db.dismiss_notification(&id).await.map_err(db_err)?;

    let envelope = AnswerEnvelope {
        id: id.clone(),
        answer: None,
        answer_label: None,
        answered_at: Some(now),
        latency_ms: Some(latency),
        renderer: notif.question_type.to_string(),
        src: notif.src.clone(),
        via: "dismissed".to_string(),
        note: None,
    };

    // Signal any blocking waiters so they return immediately
    {
        let mut waiters = state.waiters.lock().await;
        if let Some(tx) = waiters.remove(&id) {
            let _ = tx.send(envelope.clone());
        }
    }

    state.broadcaster.broadcast(&SseEvent::NotificationDismissed { id: id.clone(), envelope: envelope.clone() });

    Ok(Json(envelope))
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

/// GET /notification/{id} — get a single notification by ID
pub async fn get_notification(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Notification>, (StatusCode, Json<serde_json::Value>)> {
    match state.db.get_notification(&id).await.map_err(db_err)? {
        Some(notif) => Ok(Json(notif)),
        None => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "notification not found"})))),
    }
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
) -> Result<Json<Vec<Rule>>, (StatusCode, Json<serde_json::Value>)> {
    let now = Utc::now();
    let expires = if let Some(dur) = &body.expires_in {
        crate::models::parse_deadline(dur).ok()
    } else {
        body.expires_at.clone()
    };

    let entities = crate::entities::EntityGroups::load();
    let mut created = Vec::new();

    // ── Phase 1: Try deterministic simple compiler ────────────────────────
    let (simple_compiled, simple_duration) = crate::rules::compile(&body.text, &entities)
        .unwrap_or((serde_json::Value::Null, None));

    // If the simple compiler recognized a "mute all except" pattern, handle it
    // by generating mute-all + surface-exceptions rules.
    // We detect this by checking if the text contains "except"/"but" after "mute".
    let lower = body.text.to_lowercase();
    let is_mute_except = lower.contains("mute") &&
        (lower.contains(" except ") || lower.contains(" but ") || lower.contains(" other than "));

    if is_mute_except {
        if let Some((agent, contacts)) = crate::rules::parse_mute_except_text(&body.text, &entities) {
            let duration = simple_duration
                .as_deref()
                .and_then(|d| crate::models::parse_deadline(d).ok())
                .or(expires);

            // Create mute-all rule (low priority)
            let mute_rule = Rule {
                id: format!("r-{}", nanoid::nanoid!(6)),
                text: format!("mute all {} (auto-created by exception list)", agent),
                compiled: Some(serde_json::json!({
                    "action": "mute",
                    "match": { "agent": &agent }
                })),
                active: true,
                scope: Some(agent.clone()),
                urgency_min: 0,
                mute: true,
                priority: body.priority.unwrap_or(10),
                expires_at: duration,
                active_window: None,
                created_at: now,
            };
            state.db.insert_rule(&mute_rule).await.map_err(db_err)?;
            state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: mute_rule.clone() });
            created.push(mute_rule);

            // Create surface rules for each exception (high priority, overrides mute)
            for contact in contacts {
                let contact_clean = contact.trim().to_string();
                if contact_clean.is_empty() { continue; }
                let surface_rule = Rule {
                    id: format!("r-{}", nanoid::nanoid!(6)),
                    text: format!("surface {} from {} (exception)", agent, contact_clean),
                    compiled: Some(serde_json::json!({
                        "action": "surface",
                        "match": {
                            "agent": &agent,
                            "source_contains": [contact_clean]
                        }
                    })),
                    active: true,
                    scope: Some(agent.clone()),
                    urgency_min: 0,
                    mute: false,
                    priority: body.priority.unwrap_or(20),
                    expires_at: duration,
                    active_window: None,
                    created_at: now,
                };
                state.db.insert_rule(&surface_rule).await.map_err(db_err)?;
                state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: surface_rule.clone() });
                created.push(surface_rule);
            }

            return Ok(Json(created));
        }
    }

    // ── Phase 2: "only allow" / "allow from" patterns ──────────────────────
    if lower.starts_with("only allow ") || lower.starts_with("allow ") || lower.starts_with("only ") {
        let rest = if lower.starts_with("only allow ") {
            &body.text[11..]
        } else if lower.starts_with("allow ") {
            &body.text[6..]
        } else {
            &body.text[5..] // "only "
        };
        let rest_lower = rest.to_lowercase();

        if let Some(from_idx) = rest_lower.find(" from ") {
            let agent = rest[..from_idx].trim().to_string();
            let after_from = rest[from_idx + 6..].trim();

            let (contacts_str, duration_str) = if let Some(for_idx) = after_from.to_lowercase().rfind(" for ") {
                (&after_from[..for_idx], Some(&after_from[for_idx + 5..]))
            } else {
                (after_from, None)
            };

            let contacts = crate::rules::parse_contacts(contacts_str);
            let expanded: Vec<String> = contacts.iter()
                .flat_map(|c| entities.expand(c))
                .collect();

            if !expanded.is_empty() && !agent.is_empty() {
                let duration = duration_str
                    .and_then(|d| crate::models::parse_deadline(d).ok())
                    .or(simple_duration.as_deref().and_then(|d| crate::models::parse_deadline(d).ok()))
                    .or(expires);

                // Create mute-all rule (low priority)
                let mute_rule = Rule {
                    id: format!("r-{}", nanoid::nanoid!(6)),
                    text: format!("mute all {} (auto-created by allow list)", agent),
                    compiled: Some(serde_json::json!({
                        "action": "mute",
                        "match": { "agent": &agent }
                    })),
                    active: true,
                    scope: Some(agent.clone()),
                    urgency_min: 0,
                    mute: true,
                    priority: body.priority.unwrap_or(10),
                    expires_at: duration,
                    active_window: None,
                    created_at: now,
                };
                state.db.insert_rule(&mute_rule).await.map_err(db_err)?;
                state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: mute_rule.clone() });
                created.push(mute_rule);

                // Create surface rules for each contact (high priority, overrides mute)
                for contact in expanded {
                    let contact_clean = contact.trim().to_string();
                    if contact_clean.is_empty() { continue; }
                    let surface_rule = Rule {
                        id: format!("r-{}", nanoid::nanoid!(6)),
                        text: format!("surface {} from {}", agent, contact_clean),
                        compiled: Some(serde_json::json!({
                            "action": "surface",
                            "match": {
                                "agent": &agent,
                                "source_contains": [contact_clean]
                            }
                        })),
                        active: true,
                        scope: Some(agent.clone()),
                        urgency_min: 0,
                        mute: false,
                        priority: body.priority.unwrap_or(20),
                        expires_at: duration,
                        active_window: None,
                        created_at: now,
                    };
                    state.db.insert_rule(&surface_rule).await.map_err(db_err)?;
                    state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: surface_rule.clone() });
                    created.push(surface_rule);
                }

                return Ok(Json(created));
            }
        }
    }

    // ── Phase 3: Single rule from simple compiler ──────────────────────────
    if !simple_compiled.is_null() {
        let duration = simple_duration
            .as_deref()
            .and_then(|d| crate::models::parse_deadline(d).ok())
            .or(expires);

        let rule = Rule {
            id: format!("r-{}", nanoid::nanoid!(6)),
            text: body.text.clone(),
            compiled: Some(simple_compiled),
            active: true,
            scope: body.scope.clone(),
            urgency_min: body.urgency_min.unwrap_or(0),
            mute: body.mute.unwrap_or(false),
            priority: body.priority.unwrap_or(0),
            expires_at: duration,
            active_window: None,
            created_at: now,
        };

        state.db.insert_rule(&rule).await.map_err(db_err)?;
        state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: rule.clone() });
        created.push(rule);
        return Ok(Json(created));
    }

    // ── Phase 4: AI fallback ───────────────────────────────────────────────
    let agents = state.db.list_agents().await.map_err(db_err)?;
    let compiled = if let Some(ref router) = *state.router {
        router.compile_rule(&body.text, now, &agents).await.ok()
    } else {
        None
    };

    let rule = Rule {
        id: format!("r-{}", nanoid::nanoid!(6)),
        text: body.text.clone(),
        compiled,
        active: true,
        scope: body.scope.clone(),
        urgency_min: body.urgency_min.unwrap_or(0),
        mute: body.mute.unwrap_or(false),
        priority: body.priority.unwrap_or(0),
        expires_at: expires,
        active_window: None,
        created_at: now,
    };

    state.db.insert_rule(&rule).await.map_err(db_err)?;
    state.broadcaster.broadcast(&SseEvent::RuleCreated { rule: rule.clone() });
    created.push(rule);
    Ok(Json(created))
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
    // First check if already answered, dismissed, cancelled, or timed out
    if let Ok(Some(notif)) = state.db.get_notification(&id).await {
        match notif.status {
            NotificationStatus::Answered => {
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
            NotificationStatus::Dismissed | NotificationStatus::Cancelled | NotificationStatus::TimedOut => {
                let envelope = AnswerEnvelope {
                    id: id.clone(),
                    answer: None,
                    answer_label: None,
                    answered_at: notif.answered_at,
                    latency_ms: notif.answered_at.map(|at| at.signed_duration_since(notif.created_at).num_milliseconds()),
                    renderer: notif.question_type.to_string(),
                    src: notif.src.clone(),
                    via: format!("{:?}", notif.status).to_lowercase(),
                    note: None,
                };
                return Ok(Json(envelope));
            }
            _ => {}
        }
    }

    // Set up a oneshot waiter
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut waiters = state.waiters.lock().await;
        waiters.insert(id.clone(), tx);
    }

    // Cap the wait at the notification's deadline (if any), so `--blocking`
    // honors --deadline. Fall back to a generous ceiling otherwise.
    const MAX_WAIT: i64 = 600; // hard ceiling (seconds) for deadline-less waits
    let wait_secs: i64 = {
        let deadline = state.db.get_notification(&id).await.ok().flatten().and_then(|n| n.deadline);
        match deadline {
            Some(dl) => {
                let remaining = (dl - Utc::now()).num_seconds();
                remaining.clamp(0, MAX_WAIT)
            }
            None => MAX_WAIT,
        }
    };

    match tokio::time::timeout(std::time::Duration::from_secs(wait_secs.max(0) as u64), rx).await {
        Ok(Ok(envelope)) => Ok(Json(envelope)),
        Ok(Err(_)) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "waiter cancelled"})))),
        Err(_) => {
            // Deadline reached with no answer. Clean up the waiter, mark the
            // notification timed_out, broadcast so the dashboard reflects it,
            // and return a STRUCTURED timeout envelope (HTTP 200) so blocking
            // callers can apply their own best judgement.
            {
                let mut waiters = state.waiters.lock().await;
                waiters.remove(&id);
            }
            let notif = state.db.get_notification(&id).await.ok().flatten();
            // Only transition if still open (a human may have just answered).
            if let Some(ref n) = notif {
                if n.status == NotificationStatus::Open {
                    let _ = state.db.update_status(&id, NotificationStatus::TimedOut).await;
                }
            }
            let renderer = notif.as_ref().map(|n| n.question_type.to_string()).unwrap_or_default();
            let src = notif.as_ref().map(|n| n.src.clone()).unwrap_or_default();
            let envelope = AnswerEnvelope {
                id: id.clone(),
                answer: None,
                answer_label: None,
                answered_at: Some(Utc::now()),
                latency_ms: None,
                renderer,
                src,
                via: "timed_out".to_string(),
                note: None,
            };
            state.broadcaster.broadcast(&SseEvent::NotificationDismissed { id: id.clone(), envelope: envelope.clone() });
            Ok(Json(envelope))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<String>,
}

// ── Device token registration ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DeviceRegisterRequest {
    pub token: String,
    pub device_name: Option<String>,
}

pub async fn register_device(
    State(state): State<AppState>,
    Json(req): Json<DeviceRegisterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state.db.register_device_token(&req.token, req.device_name.as_deref())
        .await
        .map_err(db_err)?;
    tracing::info!("registered device token (first 8: {}…)", &req.token[..8.min(req.token.len())]);
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn unregister_device(
    State(state): State<AppState>,
    Json(req): Json<DeviceRegisterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state.db.unregister_device_token(&req.token)
        .await
        .map_err(db_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Push notification helper ─────────────────────────────────────────────

async fn send_push_notifications(state: &AppState, notif: &Notification) {
    let tokens = match state.db.list_device_tokens().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("failed to list device tokens: {}", e);
            return;
        }
    };
    if tokens.is_empty() {
        return;
    }

    let mut apns_guard = state.apns.lock().await;
    let apns = match apns_guard.as_mut() {
        Some(a) => a,
        None => {
            tracing::debug!("APNs not configured, skipping push");
            return;
        }
    };

    let title = if notif.urgency >= 4 {
        format!("🚨 {}", notif.agent_name)
    } else {
        notif.agent_name.clone()
    };
    let body = if notif.question.len() > 100 {
        format!("{}…", &notif.question[..100])
    } else {
        notif.question.clone()
    };

    for (token, _name) in &tokens {
        if let Err(e) = apns.send(token, &title, &body, Some(&notif.id)).await {
            tracing::warn!("push to {}… failed: {}", &token[..8.min(token.len())], e);
        }
    }
}
