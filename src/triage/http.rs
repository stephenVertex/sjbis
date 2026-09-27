use super::models::{
    CreateQueue, DecisionPatch, Observation, QueueDetail, RefreshStats, SourceKind, TriageItem,
    TriageQueue, TriageRevision,
};
use super::store::TriageStoreError;
use crate::handlers::AppState;
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashSet;

const DEFAULT_EXPORT_LIMIT: usize = 100;
const CURSOR_PREFIX: &str = "triage-v1-";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/triage/queues", post(create_queue).get(list_queues))
        .route("/triage/queues/{queue}", get(get_queue))
        .route(
            "/triage/queues/{queue}/items/{item}/decision",
            patch(record_decision),
        )
        .route("/triage/queues/{queue}/refresh", post(refresh_queue))
        .route(
            "/triage/queues/{queue}/items/{item}/attach",
            post(attach_item),
        )
        .route("/triage/queues/{queue}/close", post(close_queue))
        .route("/triage/queues/{queue}/reopen", post(reopen_queue))
        .route("/triage/queues/{queue}/export", get(export_queue))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    pub items: Vec<Observation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    pub path: String,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CatalogItem {
    pub id: String,
    pub source_kind: SourceKind,
    pub path: Option<String>,
    pub source: Option<Value>,
    pub markdown: String,
    pub content_sha256: String,
    pub captured_at: DateTime<Utc>,
    pub freshness: super::models::Freshness,
    pub latest_revision: Option<TriageRevision>,
}

impl From<TriageItem> for CatalogItem {
    fn from(item: TriageItem) -> Self {
        Self {
            id: item.id,
            source_kind: item.source_kind,
            path: item.path,
            source: item.source,
            markdown: item.markdown,
            content_sha256: item.content_sha256,
            captured_at: item.captured_at,
            freshness: item.freshness,
            latest_revision: item.latest_revision,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QueueDetailResponse {
    pub queue: TriageQueue,
    pub catalog: Vec<CatalogItem>,
}

impl From<QueueDetail> for QueueDetailResponse {
    fn from(detail: QueueDetail) -> Self {
        Self {
            queue: detail.queue,
            catalog: detail.catalog.into_iter().map(CatalogItem::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExportResponse {
    pub queue: TriageQueue,
    pub catalog: Vec<CatalogItem>,
    pub items: Vec<TriageRevision>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportQuery {
    since: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug)]
struct HttpError {
    status: StatusCode,
    body: Value,
}

impl HttpError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: json!({"error": message.into()}),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: json!({"error": message.into()}),
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<TriageStoreError> for HttpError {
    fn from(error: TriageStoreError) -> Self {
        match error {
            TriageStoreError::QueueNotFound(queue) => {
                Self::not_found(format!("triage queue not found: {queue}"))
            }
            TriageStoreError::ItemNotFound { queue_id, item_id } => {
                Self::not_found(format!("triage item not found: {queue_id}/{item_id}"))
            }
            TriageStoreError::DuplicateQueue { name, existing_id } => Self {
                status: StatusCode::CONFLICT,
                body: json!({
                    "error": format!(
                        "triage queue name already exists: {name} (queue {existing_id}); resume with `sjbis triage show {existing_id}`"
                    ),
                    "code": "duplicate_queue_name",
                    "existing_queue_id": existing_id,
                    "resume_command": format!("sjbis triage show {existing_id}"),
                }),
            },
            TriageStoreError::ClosedQueue => Self {
                status: StatusCode::CONFLICT,
                body: json!({
                    "error": "queue is closed; reopen first",
                    "code": "queue_closed",
                }),
            },
            TriageStoreError::Validation(message) => Self::bad_request(message),
            TriageStoreError::Import(error) => Self::bad_request(error.to_string()),
            TriageStoreError::Corrupt(message) => {
                tracing::error!("corrupt triage data: {message}");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({"error": format!("stored triage data is inconsistent: {message}")}),
                }
            }
            TriageStoreError::Database(error) => {
                tracing::error!("triage database error: {error}");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({"error": "triage database operation failed"}),
                }
            }
            TriageStoreError::Json(error) => {
                tracing::error!("triage JSON error: {error}");
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({"error": "stored triage JSON is invalid"}),
                }
            }
        }
    }
}

type HttpResult<T> = Result<T, HttpError>;

async fn create_queue(
    State(state): State<AppState>,
    payload: Result<Json<CreateQueue>, JsonRejection>,
) -> HttpResult<(StatusCode, Json<TriageQueue>)> {
    let request = json_payload(payload)?;
    let queue = state.db.triage().create_queue(request).await?;
    Ok((StatusCode::CREATED, Json(queue)))
}

async fn list_queues(State(state): State<AppState>) -> HttpResult<Json<Vec<TriageQueue>>> {
    Ok(Json(state.db.triage().list_queues().await?))
}

async fn get_queue(
    State(state): State<AppState>,
    Path(queue): Path<String>,
) -> HttpResult<Json<QueueDetailResponse>> {
    let detail = state
        .db
        .triage()
        .get_detail(&queue)
        .await?
        .ok_or_else(|| HttpError::not_found(format!("triage queue not found: {queue}")))?;
    Ok(Json(detail.into()))
}

async fn record_decision(
    State(state): State<AppState>,
    Path((queue, item)): Path<(String, String)>,
    payload: Result<Json<DecisionPatch>, JsonRejection>,
) -> HttpResult<Json<TriageRevision>> {
    let patch = json_payload(payload)?;
    Ok(Json(
        state
            .db
            .triage()
            .record_decision(&queue, &item, patch)
            .await?,
    ))
}

async fn refresh_queue(
    State(state): State<AppState>,
    Path(queue): Path<String>,
    payload: Result<Json<RefreshRequest>, JsonRejection>,
) -> HttpResult<Json<RefreshStats>> {
    let request = json_payload(payload)?;
    Ok(Json(
        state
            .db
            .triage()
            .refresh_queue(&queue, request.items)
            .await?,
    ))
}

async fn attach_item(
    State(state): State<AppState>,
    Path((queue, item)): Path<(String, String)>,
    payload: Result<Json<AttachRequest>, JsonRejection>,
) -> HttpResult<Json<CatalogItem>> {
    let request = json_payload(payload)?;
    let observation = Observation {
        id: item.clone(),
        markdown: request.markdown,
        path: Some(request.path),
        source: None,
    };
    let item = state
        .db
        .triage()
        .attach_item(&queue, &item, observation)
        .await?;
    Ok(Json(item.into()))
}

async fn close_queue(
    State(state): State<AppState>,
    Path(queue): Path<String>,
) -> HttpResult<Json<TriageQueue>> {
    Ok(Json(state.db.triage().close_queue(&queue).await?))
}

async fn reopen_queue(
    State(state): State<AppState>,
    Path(queue): Path<String>,
) -> HttpResult<Json<TriageQueue>> {
    Ok(Json(state.db.triage().reopen_queue(&queue).await?))
}

async fn export_queue(
    State(state): State<AppState>,
    Path(queue): Path<String>,
    query: Result<Query<ExportQuery>, QueryRejection>,
) -> HttpResult<Json<ExportResponse>> {
    let Query(query) = query.map_err(|error| HttpError::bad_request(error.body_text()))?;
    let after_event_id = decode_cursor(query.since.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_EXPORT_LIMIT);
    let store = state.db.triage();
    let detail = store
        .get_detail(&queue)
        .await?
        .ok_or_else(|| HttpError::not_found(format!("triage queue not found: {queue}")))?;
    let page = store
        .list_revisions(&detail.queue.id, after_event_id, limit)
        .await?;
    validate_export_targets(&detail, &page.items)?;
    Ok(Json(ExportResponse {
        queue: detail.queue,
        catalog: detail.catalog.into_iter().map(CatalogItem::from).collect(),
        items: page.items,
        next_cursor: encode_cursor(page.next_event_id),
    }))
}

fn json_payload<T: DeserializeOwned>(payload: Result<Json<T>, JsonRejection>) -> HttpResult<T> {
    payload
        .map(|Json(value)| value)
        .map_err(|error| HttpError::bad_request(error.body_text()))
}

fn decode_cursor(value: Option<&str>) -> HttpResult<i64> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let event_id = value
        .strip_prefix(CURSOR_PREFIX)
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .ok_or_else(|| HttpError::bad_request("invalid triage export cursor"))?;
    Ok(event_id)
}

fn encode_cursor(event_id: i64) -> String {
    format!("{CURSOR_PREFIX}{event_id}")
}

fn validate_export_targets(detail: &QueueDetail, revisions: &[TriageRevision]) -> HttpResult<()> {
    let ids = detail
        .catalog
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let revisions = revisions.iter().chain(
        detail
            .catalog
            .iter()
            .filter_map(|item| item.latest_revision.as_ref()),
    );
    for revision in revisions {
        if let Some(target) = revision.target.as_deref()
            && !ids.contains(target)
        {
            return Err(TriageStoreError::Corrupt(format!(
                "revision {} for item {} targets missing catalog item {target}",
                revision.revision, revision.item_id
            ))
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Db, router::AiRouter, sse::Broadcaster};
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request},
    };
    use chrono::TimeZone;
    use serde_json::json;
    use sqlx::PgPool;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    fn queue_fixture() -> TriageQueue {
        TriageQueue {
            id: "tq-fixture".to_string(),
            name: "fixture".to_string(),
            root: "/tmp/fixture".to_string(),
            strip_suffix: "_analysis.md".to_string(),
            source_spec: super::super::models::SourceSpec::Glob {
                patterns: vec!["**/*.md".to_string()],
            },
            status: super::super::models::QueueStatus::Open,
            complete: false,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_001, 0).unwrap(),
            closed_at: None,
            counts: super::super::models::QueueCounts {
                total: 1,
                decided: 0,
                stale: 0,
                ambiguous: 0,
            },
        }
    }

    fn revision_fixture() -> TriageRevision {
        TriageRevision {
            event_id: 7,
            queue_id: "tq-fixture".to_string(),
            item_id: "item-a".to_string(),
            revision: 2,
            verdict: Some(super::super::models::TriageVerdict::MergeInto),
            target: Some("item-b".to_string()),
            content_sha256: "a".repeat(64),
            decided_at: Utc.timestamp_opt(1_700_000_002, 0).unwrap(),
        }
    }

    fn catalog_fixture() -> CatalogItem {
        CatalogItem {
            id: "item-a".to_string(),
            source_kind: SourceKind::Path,
            path: Some("a.md".to_string()),
            source: None,
            markdown: "# A".to_string(),
            content_sha256: "a".repeat(64),
            captured_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            freshness: super::super::models::Freshness::current(),
            latest_revision: Some(revision_fixture()),
        }
    }

    #[test]
    fn serialization_locks_http_field_names_and_nulls() {
        let create: CreateQueue = serde_json::from_value(json!({
            "name": "fixture",
            "root": "/tmp/fixture",
            "strip_suffix": "_analysis.md",
            "source_spec": {"kind": "glob", "patterns": ["**/*.md"]},
            "items": [{"id": "item-a", "markdown": "# A", "path": "a.md"}]
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(create)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            ["name", "root", "strip_suffix", "source_spec", "items"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        let summary = serde_json::to_value(queue_fixture()).unwrap();
        assert_eq!(
            summary
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            [
                "id",
                "name",
                "root",
                "strip_suffix",
                "source_spec",
                "status",
                "complete",
                "created_at",
                "updated_at",
                "closed_at",
                "counts",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
        assert!(summary["closed_at"].is_null());

        let catalog = serde_json::to_value(catalog_fixture()).unwrap();
        assert_eq!(
            catalog
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            [
                "id",
                "source_kind",
                "path",
                "source",
                "markdown",
                "content_sha256",
                "captured_at",
                "freshness",
                "latest_revision",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
        assert!(catalog["source"].is_null());
        assert!(catalog.get("queue_id").is_none());
        assert_eq!(
            catalog["freshness"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            ["state", "reason", "candidate_paths"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        let detail = serde_json::to_value(QueueDetailResponse {
            queue: queue_fixture(),
            catalog: vec![catalog_fixture()],
        })
        .unwrap();
        assert_eq!(
            detail
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            ["queue", "catalog"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        let export = serde_json::to_value(ExportResponse {
            queue: queue_fixture(),
            catalog: vec![catalog_fixture()],
            items: vec![revision_fixture()],
            next_cursor: encode_cursor(7),
        })
        .unwrap();
        assert_eq!(
            export
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            ["queue", "catalog", "items", "nextCursor"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        assert_eq!(export["items"][0]["event_id"], 7);
        assert_eq!(export["items"][0]["target"], "item-b");
        assert_eq!(
            export["items"][0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            [
                "event_id",
                "queue_id",
                "item_id",
                "revision",
                "verdict",
                "target",
                "content_sha256",
                "decided_at",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );

        let refresh: RefreshRequest = serde_json::from_value(json!({
            "items": [{"id": "item-a", "markdown": "# A", "path": "a.md"}]
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(refresh).unwrap(),
            json!({
                "items": [{"id": "item-a", "markdown": "# A", "path": "a.md"}]
            })
        );
        let attach: AttachRequest = serde_json::from_value(json!({
            "path": "moved/a.md", "markdown": "# A"
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(attach).unwrap(),
            json!({
                "path": "moved/a.md", "markdown": "# A"
            })
        );
        assert_eq!(
            serde_json::to_value(RefreshStats::default())
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            [
                "current",
                "missing",
                "content_changed",
                "ambiguous",
                "added",
                "moved",
                "inline_replaced",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
    }

    #[test]
    fn decision_patch_and_cursor_validation_preserve_contract_distinctions() {
        let missing: DecisionPatch = serde_json::from_value(json!({})).unwrap();
        let cleared: DecisionPatch = serde_json::from_value(json!({"verdict": null})).unwrap();
        assert!(matches!(
            missing.verdict,
            super::super::models::PatchField::Missing
        ));
        assert!(matches!(
            cleared.verdict,
            super::super::models::PatchField::Null
        ));
        assert_eq!(serde_json::to_value(&missing).unwrap(), json!({}));
        assert_eq!(
            serde_json::to_value(&cleared).unwrap(),
            json!({"verdict": null})
        );
        assert!(serde_json::from_value::<DecisionPatch>(json!({"verdict": ""})).is_err());

        assert_eq!(decode_cursor(None).unwrap(), 0);
        assert_eq!(decode_cursor(Some("")).unwrap(), 0);
        assert_eq!(decode_cursor(Some(&encode_cursor(42))).unwrap(), 42);
        assert_eq!(
            decode_cursor(Some("42")).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );

        let item = TriageItem {
            queue_id: "tq-fixture".to_string(),
            id: "item-a".to_string(),
            source_kind: SourceKind::Inline,
            path: None,
            source: None,
            markdown: "A".to_string(),
            content_sha256: "a".repeat(64),
            captured_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            freshness: super::super::models::Freshness::current(),
            latest_revision: None,
        };
        let detail = QueueDetail {
            queue: queue_fixture(),
            catalog: vec![item],
        };
        let error = validate_export_targets(&detail, &[revision_fixture()]).unwrap_err();
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    fn test_state(pool: PgPool) -> AppState {
        AppState {
            db: Db::from_pool(pool),
            broadcaster: Broadcaster::new(),
            router: Arc::new(None::<AiRouter>),
            waiters: Arc::new(Mutex::new(HashMap::new())),
            apns: Arc::new(Mutex::new(None)),
        }
    }

    async fn request(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let response = app
            .clone()
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs DATABASE_URL: run against a disposable PostgreSQL database"]
    async fn routes_cover_queue_lifecycle_and_incremental_export(pool: PgPool) {
        let app = routes().with_state(test_state(pool));
        let create = json!({
            "name": "http-fixture",
            "root": "/tmp/http-fixture",
            "strip_suffix": "_analysis.md",
            "source_spec": {"kind": "glob", "patterns": ["**/*.md"]},
            "items": [
                {"id": "a", "markdown": "A", "path": "a.md"},
                {"id": "b", "markdown": "B", "path": "b.md"}
            ]
        });
        let (status, created) =
            request(&app, Method::POST, "/triage/queues", Some(create.clone())).await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(created.get("nextCursor").is_none());
        let queue = created["id"].as_str().unwrap();

        let (status, listed) = request(&app, Method::GET, "/triage/queues", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        let (status, detail) =
            request(&app, Method::GET, &format!("/triage/queues/{queue}"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["catalog"].as_array().unwrap().len(), 2);

        let (status, duplicate) = request(&app, Method::POST, "/triage/queues", Some(create)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(duplicate["existing_queue_id"], queue);
        assert!(
            duplicate["resume_command"]
                .as_str()
                .unwrap()
                .contains(queue)
        );

        let decision_uri = format!("/triage/queues/{queue}/items/a/decision");
        let (status, _) = request(
            &app,
            Method::PATCH,
            &decision_uri,
            Some(json!({"verdict": "merge_into", "target": "unknown"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = request(
            &app,
            Method::PATCH,
            &format!("/triage/queues/{queue}/items/unknown/decision"),
            Some(json!({"verdict": "delete"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, export_after_rejection) = request(
            &app,
            Method::GET,
            &format!("/triage/queues/{queue}/export"),
            None,
        )
        .await;
        assert!(
            export_after_rejection["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let (status, revision) = request(
            &app,
            Method::PATCH,
            &decision_uri,
            Some(json!({"verdict": "merge_into", "target": "b"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(revision["revision"], 1);

        let (status, _) = request(
            &app,
            Method::PATCH,
            &decision_uri,
            Some(json!({"verdict": ""})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, stats) = request(
            &app,
            Method::POST,
            &format!("/triage/queues/{queue}/refresh"),
            Some(json!({"items": [{"id": "a", "markdown": "A", "path": "a.md"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["missing"], 1);

        let (status, attached) = request(
            &app,
            Method::POST,
            &format!("/triage/queues/{queue}/items/b/attach"),
            Some(json!({"path": "moved/b.md", "markdown": "B"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(attached["path"], "moved/b.md");

        let (status, closed) = request(
            &app,
            Method::POST,
            &format!("/triage/queues/{queue}/close"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(closed["status"], "closed");
        let (status, _) = request(
            &app,
            Method::PATCH,
            &format!("/triage/queues/{queue}/items/b/decision"),
            Some(json!({"verdict": "delete"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (status, _) = request(
            &app,
            Method::POST,
            &format!("/triage/queues/{queue}/reopen"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = request(
            &app,
            Method::PATCH,
            &format!("/triage/queues/{queue}/items/b/decision"),
            Some(json!({"verdict": "delete"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let export_uri = format!("/triage/queues/{queue}/export?limit=1");
        let (status, first) = request(&app, Method::GET, &export_uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(first["catalog"].as_array().unwrap().len(), 2);
        let cursor = first["nextCursor"].as_str().unwrap();
        let (_, from_empty) = request(
            &app,
            Method::GET,
            &format!("/triage/queues/{queue}/export?since=&limit=1"),
            None,
        )
        .await;
        assert_eq!(from_empty["items"], first["items"]);
        let (_, second) = request(
            &app,
            Method::GET,
            &format!("/triage/queues/{queue}/export?since={cursor}&limit=1"),
            None,
        )
        .await;
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_ne!(
            second["items"][0]["event_id"],
            first["items"][0]["event_id"]
        );

        let (status, _) = request(&app, Method::GET, "/triage/queues/unknown", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
