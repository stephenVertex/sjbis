//! HTTP endpoints for ta grants (see README "Grants (ta)"). Verification and
//! every decision that can be made without I/O live in `crate::grants`.

use crate::grants::{
    GrantError, GrantStatus, GrantView, Record, Registry, StoredGrant, check_claim,
    grant_answer_envelope, registry_path, verify_envelope,
};
use crate::handlers::{AppState, deliver_response};
use crate::models::{AnswerEnvelope, SseEvent};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

type ApiError = (StatusCode, Json<Value>);

fn grant_err(e: GrantError) -> ApiError {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(e.body()))
}

fn db_err<E: std::fmt::Display>(e: E) -> ApiError {
    tracing::error!("grants db error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// POST /grants: verify a signed envelope against the signer registry
/// (re-read on every request), store it, and apply its effect: a grant
/// answers its approval ask and wakes blocking waiters, a revoke marks its
/// target, an enroll is stored only.
pub async fn post_grant(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<(StatusCode, Json<GrantView>), ApiError> {
    let raw: Value = serde_json::from_slice(&body)
        .map_err(|e| grant_err(GrantError::Malformed(format!("body is not JSON: {e}"))))?;
    let registry = Registry::load(&registry_path()).map_err(|e| {
        tracing::error!("signer registry: {e}");
        grant_err(GrantError::Registry(e))
    })?;
    if registry.is_empty() {
        tracing::warn!(
            "signer registry {} is empty or missing; every grant will be refused",
            registry_path().display()
        );
    } else {
        tracing::debug!("signer registry: {} signer(s)", registry.len());
    }
    let verified = verify_envelope(&raw, &registry).map_err(|e| {
        tracing::warn!("POST /grants refused: {e}");
        grant_err(e)
    })?;
    let now = Utc::now();
    let stored = StoredGrant::from_verified(&verified, now);

    match verified.record() {
        Record::Grant(record) => {
            let notif = state
                .db
                .insert_grant_answering(&stored, record, now)
                .await
                .map_err(db_err)?
                .map_err(grant_err)?;
            let envelope = grant_answer_envelope(&notif, record, &verified.envelope, now);
            {
                let mut waiters = state.waiters.lock().await;
                if let Some(tx) = waiters.remove(&notif.id) {
                    let _ = tx.send(envelope.clone());
                }
            }
            state
                .broadcaster
                .broadcast(&SseEvent::NotificationAnswered {
                    envelope: envelope.clone(),
                });
            let answer = envelope.answer.clone().unwrap_or_default();
            crate::push_handlers::push_closed(&state, &notif.id, &notif.agent_name, &format!("{} \u{00b7} {} by {}", record.decision.as_str(), stored.id, record.signer));
            tokio::spawn(deliver_response(notif, answer, envelope));
        }
        Record::Revoke(record) => {
            state
                .db
                .insert_grant_revoking(&stored, &record.target)
                .await
                .map_err(db_err)?
                .map_err(grant_err)?;
        }
        Record::Enroll(_) => {
            state
                .db
                .insert_grant_only(&stored)
                .await
                .map_err(db_err)?
                .map_err(grant_err)?;
        }
    }
    tracing::info!(
        "stored {} {} signed by {}",
        stored.kind,
        stored.id,
        stored.signer
    );
    Ok((StatusCode::CREATED, Json(stored.view_at(now))))
}

async fn load_grant(state: &AppState, id: &str) -> Result<StoredGrant, ApiError> {
    state
        .db
        .get_grant(id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| grant_err(GrantError::GrantNotFound(id.to_string())))
}

/// GET /grants/{id}: `{envelope, status}`.
pub async fn get_grant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<GrantView>, ApiError> {
    Ok(Json(load_grant(&state, &id).await?.view_at(Utc::now())))
}

/// GET /grants/{id}/status: `{expired, revoked_by, claimed_by, claimed_at}`.
pub async fn get_grant_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<GrantStatus>, ApiError> {
    Ok(Json(load_grant(&state, &id).await?.status_at(Utc::now())))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub ask: Option<String>,
    pub renews: Option<String>,
    /// RFC 3339 timestamp: grants issued at or after it, newest first (`ta log`).
    pub since: Option<String>,
    /// With `since`: only grants whose record says this agent asked.
    pub agent: Option<String>,
    /// With `since`: cap on the number of rows (default 200, max 1000).
    pub limit: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ListFilter {
    Ask(String),
    Renews(String),
    Since {
        since: DateTime<Utc>,
        agent: Option<String>,
        limit: u32,
    },
}

const LIST_DEFAULT_LIMIT: u32 = 200;
const LIST_MAX_LIMIT: u32 = 1000;

/// Exactly one of `ask`, `renews` or `since`, non-empty. `agent` and `limit`
/// only qualify `since`.
pub fn list_filter(q: ListQuery) -> Result<ListFilter, GrantError> {
    let malformed = || {
        GrantError::Malformed(
            "pass exactly one non-empty query parameter: ask=<notification id>, renews=<grant id> or since=<rfc3339>"
                .to_string(),
        )
    };
    match (q.ask, q.renews, q.since) {
        (Some(a), None, None) if !a.is_empty() => Ok(ListFilter::Ask(a)),
        (None, Some(r), None) if !r.is_empty() => Ok(ListFilter::Renews(r)),
        (None, None, Some(t)) if !t.is_empty() => {
            let since = DateTime::parse_from_rfc3339(&t)
                .map_err(|e| GrantError::Malformed(format!("since: not an RFC 3339 timestamp ({e})")))?
                .with_timezone(&Utc);
            let agent = q.agent.filter(|a| !a.is_empty());
            let limit = q.limit.unwrap_or(LIST_DEFAULT_LIMIT).clamp(1, LIST_MAX_LIMIT);
            Ok(ListFilter::Since { since, agent, limit })
        }
        _ => Err(malformed()),
    }
}

/// GET /grants?ask=ID or GET /grants?renews=ID: array of `{envelope, status}`.
pub async fn list_grants(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<GrantView>>, ApiError> {
    let grants = match list_filter(q).map_err(grant_err)? {
        ListFilter::Ask(id) => state.db.list_grants_by_notification(&id).await,
        ListFilter::Renews(id) => state.db.list_grants_by_renews(&id).await,
        ListFilter::Since { since, agent, limit } => state.db.list_grants_since(since, agent.as_deref(), limit).await,
    }
    .map_err(db_err)?;
    let now = Utc::now();
    Ok(Json(grants.iter().map(|g| g.view_at(now)).collect()))
}

/// Parse a claim body `{"by": "<agent>"}`.
pub fn parse_claim_body(body: &[u8]) -> Result<String, GrantError> {
    #[derive(Deserialize)]
    struct ClaimBody {
        by: String,
    }
    let parsed: ClaimBody = serde_json::from_slice(body).map_err(|e| {
        GrantError::Malformed(format!("claim body must be {{\"by\": \"...\"}}: {e}"))
    })?;
    if parsed.by.trim().is_empty() {
        return Err(GrantError::Malformed("claim `by` is empty".to_string()));
    }
    Ok(parsed.by)
}

/// POST /grants/{id}/claim {by}: consume a once grant. 409 if it is not a
/// once grant or is already claimed.
pub async fn claim_grant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<GrantView>, ApiError> {
    let by = parse_claim_body(&body).map_err(grant_err)?;
    let grant = load_grant(&state, &id).await?;
    check_claim(&grant).map_err(grant_err)?;
    let now = Utc::now();
    if !state.db.claim_grant(&id, &by, now).await.map_err(db_err)? {
        return Err(grant_err(GrantError::AlreadyClaimed(id)));
    }
    let grant = load_grant(&state, &id).await?;
    Ok(Json(grant.view_at(now)))
}

/// For GET /wait on an approval ask that is already answered: attach the
/// grant that answered it (`via = "ta:<signer>"`, `grant = envelope`).
pub async fn attach_answering_grant(state: &AppState, envelope: &mut AnswerEnvelope) {
    match state.db.list_grants_by_notification(&envelope.id).await {
        Ok(grants) => {
            if let Some(g) = grants.iter().rev().find(|g| g.kind == "grant") {
                g.attach_to_answer(envelope);
            }
        }
        Err(e) => tracing::error!("grants lookup for {}: {e}", envelope.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_filter_requires_exactly_one_param() {
        let q = |ask: Option<&str>, renews: Option<&str>, since: Option<&str>| ListQuery {
            ask: ask.map(String::from),
            renews: renews.map(String::from),
            since: since.map(String::from),
            ..Default::default()
        };
        assert_eq!(
            list_filter(q(Some("sjbis-1"), None, None)),
            Ok(ListFilter::Ask("sjbis-1".into()))
        );
        assert_eq!(
            list_filter(q(None, Some("ta-abcdef"), None)),
            Ok(ListFilter::Renews("ta-abcdef".into()))
        );
        let t = "2026-10-23T20:00:00Z";
        assert_eq!(
            list_filter(q(None, None, Some(t))),
            Ok(ListFilter::Since {
                since: DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc),
                agent: None,
                limit: LIST_DEFAULT_LIMIT,
            })
        );
        let with_agent = ListQuery { agent: Some("deploybot".into()), limit: Some(5000), ..q(None, None, Some(t)) };
        match list_filter(with_agent).unwrap() {
            ListFilter::Since { agent, limit, .. } => {
                assert_eq!(agent.as_deref(), Some("deploybot"));
                assert_eq!(limit, LIST_MAX_LIMIT);
            }
            other => panic!("unexpected {other:?}"),
        }
        for bad in [
            q(None, None, None),
            q(Some("a"), Some("b"), None),
            q(Some("a"), None, Some(t)),
            q(Some(""), None, None),
            q(None, Some(""), None),
            q(None, None, Some("")),
            q(None, None, Some("yesterday")),
        ] {
            let err = list_filter(bad).unwrap_err();
            assert_eq!(err.http_status(), 400);
        }
    }

    #[test]
    fn claim_body_parsing() {
        assert_eq!(
            parse_claim_body(br#"{"by":"agent-y"}"#),
            Ok("agent-y".into())
        );
        for bad in [
            &b""[..],
            b"{}",
            br#"{"by":""}"#,
            br#"{"by":"  "}"#,
            br#"{"by":3}"#,
            b"nope",
        ] {
            assert_eq!(parse_claim_body(bad).unwrap_err().http_status(), 400);
        }
    }

    #[test]
    fn grant_err_maps_status_and_body() {
        let (status, Json(body)) = grant_err(GrantError::QuestionMismatch);
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], "question_mismatch");
        let (status, _) = grant_err(GrantError::IdCollision("ta-abcdef".into()));
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = grant_err(GrantError::Malformed("x".into()));
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
