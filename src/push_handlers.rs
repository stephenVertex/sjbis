//! HTTP side of the phone web app: device registration, Web Push
//! subscriptions, the signer registry listing, and the hooks the ask
//! lifecycle calls to push.
//!
//! - `GET  /push/vapid`        -> `{enabled, public_key, subject}`
//! - `POST /push/register`     -> upsert a device (`device_id`, optional
//!   `subscription {endpoint, keys{p256dh, auth}}`, `device_name`,
//!   `signer_id`, `pubkey`, `user_agent`); returns `{ok, device_id, enrolled}`
//! - `POST /push/unregister`   -> `{device_id}` forgets the device
//! - `GET  /push/devices`      -> devices without their push secrets
//! - `POST /push/test`         -> `{device_id?}` sends a test push
//! - `GET  /signers`           -> the pinned signer registry (public data)

use crate::grants::registry_path;
use crate::handlers::AppState;
use crate::models::Notification;
use crate::webpush::{MAX_PAYLOAD, Outcome, Subscription, WebPush};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

type ApiError = (StatusCode, Json<Value>);

fn bad(msg: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.into(), "code": "bad_request" })),
    )
}

fn db_err<E: std::fmt::Display>(e: E) -> ApiError {
    tracing::error!("db error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
}

/// A stored device row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
    pub device_name: Option<String>,
    pub signer_id: Option<String>,
    pub pubkey: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_push_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub device_id: String,
    #[serde(default)]
    pub subscription: Option<RawSubscription>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub signer_id: Option<String>,
    #[serde(default)]
    pub pubkey: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

/// `PushSubscription.toJSON()` as the browser produces it.
#[derive(Debug, Deserialize)]
pub struct RawSubscription {
    pub endpoint: String,
    pub keys: RawKeys,
}

#[derive(Debug, Deserialize)]
pub struct RawKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceRef {
    #[serde(default)]
    pub device_id: Option<String>,
}

/// Validate a registration body into the stored shape.
pub fn parse_register(
    req: RegisterRequest,
) -> Result<(String, Option<Subscription>, RegisterRequest), String> {
    let id = req.device_id.trim().to_string();
    if id.is_empty()
        || id.len() > 100
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("device_id must be 1-100 chars of [A-Za-z0-9_-]".into());
    }
    let sub = match &req.subscription {
        None => None,
        Some(s) => {
            if !s.endpoint.starts_with("https://") {
                return Err("subscription.endpoint must be an https URL".into());
            }
            if s.keys.p256dh.is_empty() || s.keys.auth.is_empty() {
                return Err("subscription.keys.p256dh and .auth are required".into());
            }
            Some(Subscription {
                endpoint: s.endpoint.clone(),
                p256dh: s.keys.p256dh.clone(),
                auth: s.keys.auth.clone(),
            })
        }
    };
    if req
        .signer_id
        .as_deref()
        .is_some_and(|sid| sid.trim().is_empty() || sid.len() > 80)
    {
        return Err("signer_id must be 1-80 chars".into());
    }
    if let Some(pk) = &req.pubkey {
        crate::grants::decode_pubkey(pk).map_err(|e| format!("pubkey: {e}"))?;
    }
    Ok((id, sub, req))
}

/// Is `signer_id` pinned in the registry with exactly `pubkey`?
pub fn enrolled(signer_id: Option<&str>, pubkey: Option<&str>) -> bool {
    let (Some(sid), Some(pk)) = (signer_id, pubkey) else {
        return false;
    };
    match crate::grants::Registry::load(&registry_path()) {
        Ok(reg) => reg
            .get(sid)
            .map(|e| e.signer.pubkey.trim() == pk.trim())
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub async fn vapid(State(state): State<AppState>) -> Json<Value> {
    match state.webpush.as_ref() {
        Some(wp) => Json(
            json!({ "enabled": true, "public_key": wp.public_key_b64url(), "subject": wp.subject() }),
        ),
        None => Json(json!({ "enabled": false })),
    }
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<Value>, ApiError> {
    let (id, sub, req) = parse_register(req).map_err(bad)?;
    let dev = state
        .db
        .upsert_push_device(
            &id,
            sub.as_ref(),
            req.device_name.as_deref(),
            req.signer_id.as_deref(),
            req.pubkey.as_deref(),
            req.user_agent.as_deref(),
        )
        .await
        .map_err(db_err)?;
    tracing::info!(
        "phone device {} registered (signer {}, push {})",
        dev.device_id,
        dev.signer_id.as_deref().unwrap_or("-"),
        if dev.subscription.is_some() {
            "yes"
        } else {
            "no"
        }
    );
    let enrolled = enrolled(dev.signer_id.as_deref(), dev.pubkey.as_deref());
    Ok(Json(
        json!({ "ok": true, "device_id": dev.device_id, "enrolled": enrolled, "push": dev.subscription.is_some() }),
    ))
}

pub async fn unregister(
    State(state): State<AppState>,
    Json(req): Json<DeviceRef>,
) -> Result<Json<Value>, ApiError> {
    let id = req.device_id.ok_or_else(|| bad("device_id is required"))?;
    let removed = state.db.delete_push_device(&id).await.map_err(db_err)?;
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

pub async fn devices(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let devs = state.db.list_push_devices().await.map_err(db_err)?;
    let out: Vec<Value> = devs
        .iter()
        .map(|d| {
            json!({
                "device_id": d.device_id,
                "device_name": d.device_name,
                "signer_id": d.signer_id,
                "pubkey": d.pubkey,
                "enrolled": enrolled(d.signer_id.as_deref(), d.pubkey.as_deref()),
                "push": d.subscription.is_some(),
                "push_host": d.subscription.as_ref().and_then(|s| reqwest::Url::parse(&s.endpoint).ok()).and_then(|u| u.host_str().map(String::from)),
                "user_agent": d.user_agent,
                "created_at": d.created_at,
                "updated_at": d.updated_at,
                "last_push_at": d.last_push_at,
                "last_error": d.last_error,
            })
        })
        .collect();
    Ok(Json(Value::Array(out)))
}

pub async fn test_push(
    State(state): State<AppState>,
    body: Option<Json<DeviceRef>>,
) -> Result<Json<Value>, ApiError> {
    let Some(wp) = state.webpush.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({ "error": "web push is not enabled on this daemon", "code": "push_disabled" }),
            ),
        ));
    };
    let only = body.and_then(|b| b.0.device_id);
    let devs = state.db.list_push_devices().await.map_err(db_err)?;
    let payload = json!({
        "kind": "test",
        "id": format!("test-{}", Utc::now().timestamp()),
        "title": "sjbis",
        "body": format!("test push from the daemon at {}", Utc::now().format("%H:%M:%S UTC")),
    });
    let mut results = Vec::new();
    for d in devs
        .iter()
        .filter(|d| only.as_deref().is_none_or(|o| o == d.device_id))
    {
        let Some(sub) = d.subscription.as_ref() else {
            results.push(json!({ "device_id": d.device_id, "result": "no subscription" }));
            continue;
        };
        let outcome = wp
            .send(sub, payload.to_string().as_bytes(), 300, None)
            .await;
        record_outcome(&state, &d.device_id, &outcome).await;
        results.push(json!({ "device_id": d.device_id, "result": describe(&outcome) }));
    }
    Ok(Json(
        json!({ "sent": results.iter().filter(|r| r["result"] == "sent").count(), "results": results }),
    ))
}

pub async fn signers() -> Result<Json<Value>, ApiError> {
    let reg = crate::grants::Registry::load(&registry_path()).map_err(|e| {
        tracing::error!("signer registry: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("signer registry: {e}") })),
        )
    })?;
    let out: Vec<Value> = reg
        .signers()
        .map(|s| json!({ "id": s.id, "method": s.method, "device": s.device, "pubkey": s.pubkey, "added": s.added }))
        .collect();
    Ok(Json(Value::Array(out)))
}

fn describe(o: &Outcome) -> String {
    match o {
        Outcome::Sent => "sent".into(),
        Outcome::Gone => "gone (subscription removed)".into(),
        Outcome::Failed(m) => format!("failed: {m}"),
    }
}

async fn record_outcome(state: &AppState, device_id: &str, outcome: &Outcome) {
    let r = match outcome {
        Outcome::Sent => state.db.mark_push_ok(device_id).await,
        Outcome::Gone => {
            state
                .db
                .clear_push_subscription(device_id, "push service says the subscription is gone")
                .await
        }
        Outcome::Failed(m) => state.db.mark_push_error(device_id, m).await,
    };
    if let Err(e) = r {
        tracing::warn!("recording push outcome for {device_id}: {e}");
    }
}

/// The payload for a new ask. Title is the agent, body the question.
pub fn ask_payload(n: &Notification) -> Value {
    let title = if n.urgency >= 4 {
        format!("\u{1F6A8} {}", n.agent_name)
    } else {
        n.agent_name.clone()
    };
    let body = truncate_chars(&n.question, 180);
    let mut v = json!({
        "kind": "ask",
        "id": n.id,
        "title": title,
        "body": body,
        "type": n.question_type.to_string(),
        "urgency": n.urgency,
        "agent": n.agent_name,
    });
    if let Some(d) = &n.deadline {
        v["deadline"] = json!(d);
    }
    v
}

/// The payload that tells the phone an ask is no longer open.
pub fn closed_payload(id: &str, agent: &str, what: &str) -> Value {
    json!({ "kind": "closed", "id": id, "title": agent, "body": what })
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('\u{2026}');
    }
    out
}

/// Push-service TTL for an ask: until its deadline, else a day.
fn ask_ttl(n: &Notification) -> u32 {
    match n.deadline {
        Some(d) => (d - Utc::now()).num_seconds().clamp(60, 86_400) as u32,
        None => 86_400,
    }
}

/// Fan a payload out to every subscribed device (spawned; never blocks the request).
pub fn push_all(state: &AppState, payload: Value, ttl: u32, topic: Option<String>) {
    let Some(wp) = state.webpush.as_ref().clone() else {
        return;
    };
    let state = state.clone();
    tokio::spawn(async move {
        let devs = match state.db.list_push_devices().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("web push: listing devices: {e}");
                return;
            }
        };
        let bytes = payload.to_string().into_bytes();
        if bytes.len() > MAX_PAYLOAD {
            tracing::warn!(
                "web push: payload {} bytes exceeds {MAX_PAYLOAD}; skipped",
                bytes.len()
            );
            return;
        }
        for d in devs.iter() {
            let Some(sub) = d.subscription.as_ref() else {
                continue;
            };
            let outcome = wp.send(sub, &bytes, ttl, topic.as_deref()).await;
            match &outcome {
                Outcome::Sent => tracing::info!(
                    "web push -> {} ({}): sent",
                    d.device_id,
                    d.device_name.as_deref().unwrap_or("-")
                ),
                Outcome::Gone => {
                    tracing::warn!("web push -> {}: subscription gone; cleared", d.device_id)
                }
                Outcome::Failed(m) => tracing::warn!("web push -> {}: {m}", d.device_id),
            }
            record_outcome(&state, &d.device_id, &outcome).await;
        }
    });
}

/// A new open ask: push it to every phone.
pub fn push_created(state: &AppState, n: &Notification) {
    push_all(state, ask_payload(n), ask_ttl(n), Some(n.id.clone()));
}

/// An ask stopped being open (answered, cancelled, dismissed): tell the phones
/// so a banner that is still showing can be replaced.
pub fn push_closed(state: &AppState, id: &str, agent: &str, what: &str) {
    push_all(
        state,
        closed_payload(id, agent, what),
        600,
        Some(id.to_string()),
    );
}

/// Shared handle type on `AppState`.
pub type WebPushHandle = Arc<Option<Arc<WebPush>>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn req(json: Value) -> RegisterRequest {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn register_validation() {
        let ok = parse_register(req(
            json!({ "device_id": "abc-123", "device_name": "iPhone" }),
        ))
        .unwrap();
        assert_eq!(ok.0, "abc-123");
        assert!(ok.1.is_none());
        assert!(parse_register(req(json!({ "device_id": "" }))).is_err());
        assert!(parse_register(req(json!({ "device_id": "has space" }))).is_err());
        assert!(parse_register(req(json!({ "device_id": "x", "subscription": { "endpoint": "http://plain", "keys": { "p256dh": "a", "auth": "b" } } }))).is_err());
        assert!(parse_register(req(json!({ "device_id": "x", "pubkey": "not a key" }))).is_err());
        let with_sub = parse_register(req(json!({ "device_id": "x", "subscription": { "endpoint": "https://web.push.apple.com/abc", "keys": { "p256dh": "BAAA", "auth": "zz" } } }))).unwrap();
        assert_eq!(
            with_sub.1.unwrap().endpoint,
            "https://web.push.apple.com/abc"
        );
    }

    #[test]
    fn payload_shapes() {
        let n: Notification = serde_json::from_value(json!({
            "id": "sjbis-abc", "agent_name": "agent-x", "sender": "agent-x", "src": "agent-x",
            "question": "Delete the staging database? ".repeat(20), "question_type": "approval", "urgency": 5,
            "blocking": true, "deadline": null, "reply_to": "stdout", "status": "open",
            "created_at": "2026-10-23T01:12:00Z", "ttl_secs": 3600
        })).unwrap();
        let p = ask_payload(&n);
        assert_eq!(p["kind"], "ask");
        assert_eq!(p["id"], "sjbis-abc");
        assert!(p["title"].as_str().unwrap().contains("agent-x"));
        assert!(p["body"].as_str().unwrap().chars().count() <= 181);
        assert!(p["body"].as_str().unwrap().ends_with('\u{2026}'));
        assert!(p.to_string().len() < MAX_PAYLOAD);
        let c = closed_payload("sjbis-abc", "agent-x", "answered: Approved");
        assert_eq!(c["kind"], "closed");
        assert_eq!(c["body"], "answered: Approved");
    }

    #[test]
    fn enrolled_needs_both_and_a_registry_match() {
        assert!(!enrolled(None, Some("BA")));
        assert!(!enrolled(Some("x"), None));
    }
}
