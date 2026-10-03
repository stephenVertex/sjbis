use super::import::{
    ImportError, ReconcileAction, attachment_action, reconcile, validate_and_sort_observations,
    validate_persisted_source_spec,
};
use super::models::*;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};

const QUEUE_SUMMARY_SQL: &str = r#"
    SELECT
        q.id, q.name, q.root, q.strip_suffix, q.source_spec, q.status,
        q.created_at, q.updated_at, q.closed_at,
        COUNT(i.id)::BIGINT AS total,
        COUNT(*) FILTER (WHERE latest.verdict IS NOT NULL)::BIGINT AS decided,
        COUNT(*) FILTER (WHERE i.freshness_state <> 'current')::BIGINT AS stale,
        COUNT(*) FILTER (WHERE i.freshness_state = 'ambiguous')::BIGINT AS ambiguous
    FROM triage_queues q
    LEFT JOIN triage_items i ON i.queue_id = q.id
    LEFT JOIN LATERAL (
        SELECT r.verdict
        FROM triage_revisions r
        WHERE r.queue_id = i.queue_id AND r.item_id = i.id
        ORDER BY r.revision DESC
        LIMIT 1
    ) latest ON TRUE
"#;

const ITEM_DETAIL_SQL: &str = r#"
    SELECT
        i.queue_id, i.id, i.source_kind, i.source_path, i.source,
        i.markdown, i.content_sha256, i.captured_at,
        i.freshness_state, i.freshness_reason, i.candidate_paths,
        latest.event_id, latest.revision, latest.verdict, latest.target,
        latest.revision_content_sha256, latest.decided_at
    FROM triage_items i
    LEFT JOIN LATERAL (
        SELECT
            r.event_id, r.revision, r.verdict, r.target,
            r.content_sha256 AS revision_content_sha256, r.decided_at
        FROM triage_revisions r
        WHERE r.queue_id = i.queue_id AND r.item_id = i.id
        ORDER BY r.revision DESC
        LIMIT 1
    ) latest ON TRUE
"#;

#[derive(Debug, thiserror::Error)]
pub enum TriageStoreError {
    #[error("triage queue not found: {0}")]
    QueueNotFound(String),
    #[error("triage item not found: {queue_id}/{item_id}")]
    ItemNotFound { queue_id: String, item_id: String },
    #[error(
        "triage queue name already exists: {name} (queue {existing_id}); resume with `sjbis triage show {existing_id}`"
    )]
    DuplicateQueue { name: String, existing_id: String },
    #[error("queue is closed; reopen first")]
    ClosedQueue,
    #[error("{0}")]
    Validation(String),
    #[error("stored triage data is inconsistent: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Import(#[from] ImportError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct TriageStore {
    pool: PgPool,
}

impl TriageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_queue(
        &self,
        request: CreateQueue,
    ) -> Result<TriageQueue, TriageStoreError> {
        validate_create_request(&request)?;
        let items = validate_and_sort_observations(request.items)?;
        let queue_id = format!("tq-{}", nanoid::nanoid!(10));
        let now = Utc::now();
        let source_spec = serde_json::to_value(&request.source_spec)?;
        let mut transaction = self.pool.begin().await?;

        let insert = sqlx::query(
            r#"INSERT INTO triage_queues
                   (id, name, root, strip_suffix, source_spec, status, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, 'open', $6, $6)"#,
        )
        .bind(&queue_id)
        .bind(&request.name)
        .bind(&request.root)
        .bind(&request.strip_suffix)
        .bind(source_spec)
        .bind(now)
        .execute(&mut *transaction)
        .await;

        if let Err(error) = insert {
            let duplicate = is_unique_violation(&error);
            transaction.rollback().await?;
            if duplicate {
                let existing_id =
                    sqlx::query_scalar::<_, String>("SELECT id FROM triage_queues WHERE name = $1")
                        .bind(&request.name)
                        .fetch_one(&self.pool)
                        .await?;
                return Err(TriageStoreError::DuplicateQueue {
                    name: request.name,
                    existing_id,
                });
            }
            return Err(error.into());
        }

        for observation in &items {
            insert_item(&mut transaction, &queue_id, observation, now).await?;
        }
        transaction.commit().await?;
        self.get_queue(&queue_id)
            .await?
            .ok_or_else(|| TriageStoreError::QueueNotFound(queue_id))
    }

    pub async fn list_queues(&self) -> Result<Vec<TriageQueue>, TriageStoreError> {
        let query = format!(
            "{} GROUP BY q.id ORDER BY q.updated_at DESC, q.id",
            QUEUE_SUMMARY_SQL
        );
        let rows = sqlx::query(&query).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_queue).collect()
    }

    pub async fn get_queue(
        &self,
        queue_reference: &str,
    ) -> Result<Option<TriageQueue>, TriageStoreError> {
        let query = format!(
            "{} WHERE q.id = $1 OR q.name = $1 GROUP BY q.id ORDER BY (q.id = $1) DESC LIMIT 1",
            QUEUE_SUMMARY_SQL
        );
        let row = sqlx::query(&query)
            .bind(queue_reference)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_queue).transpose()
    }

    pub async fn get_detail(
        &self,
        queue_reference: &str,
    ) -> Result<Option<QueueDetail>, TriageStoreError> {
        let Some(queue) = self.get_queue(queue_reference).await? else {
            return Ok(None);
        };
        let catalog = self.load_items(&queue.id).await?;
        Ok(Some(QueueDetail { queue, catalog }))
    }

    pub async fn record_decision(
        &self,
        queue_reference: &str,
        item_id: &str,
        patch: DecisionPatch,
    ) -> Result<TriageRevision, TriageStoreError> {
        let mut transaction = self.pool.begin().await?;
        let queue_id = lock_open_queue(&mut transaction, queue_reference).await?;
        let item = load_item_for_update(&mut transaction, &queue_id, item_id).await?;
        let previous = load_latest_revision(&mut transaction, &queue_id, item_id).await?;

        let verdict = match patch.verdict {
            PatchField::Missing => previous.as_ref().and_then(|revision| revision.verdict),
            PatchField::Null => None,
            PatchField::Value(verdict) => Some(verdict),
        };
        let target_patch = patch.target;
        let target = if verdict == Some(TriageVerdict::MergeInto) {
            match target_patch {
                PatchField::Missing => previous.and_then(|revision| revision.target),
                PatchField::Null => None,
                PatchField::Value(target) => Some(validate_target_text(target)?),
            }
        } else {
            if let PatchField::Value(target) = target_patch {
                return Err(TriageStoreError::Validation(format!(
                    "target {target:?} is only valid for merge_into"
                )));
            }
            None
        };

        validate_decision(
            &mut transaction,
            &queue_id,
            item_id,
            verdict,
            target.as_deref(),
        )
        .await?;

        let revision = sqlx::query_scalar::<_, i64>(
            r#"SELECT COALESCE(MAX(revision), 0) + 1
               FROM triage_revisions
               WHERE queue_id = $1 AND item_id = $2"#,
        )
        .bind(&queue_id)
        .bind(item_id)
        .fetch_one(&mut *transaction)
        .await?;
        let now = Utc::now();
        let row = sqlx::query(
            r#"INSERT INTO triage_revisions
                   (queue_id, item_id, revision, verdict, target, content_sha256, decided_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING event_id, queue_id, item_id, revision, verdict, target,
                         content_sha256, decided_at"#,
        )
        .bind(&queue_id)
        .bind(item_id)
        .bind(revision)
        .bind(verdict.map(TriageVerdict::as_str))
        .bind(&target)
        .bind(&item.content_sha256)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("UPDATE triage_queues SET updated_at = $1 WHERE id = $2")
            .bind(now)
            .bind(&queue_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        row_to_revision(&row)
    }

    pub async fn refresh_queue(
        &self,
        queue_reference: &str,
        observations: Vec<Observation>,
    ) -> Result<RefreshStats, TriageStoreError> {
        let mut transaction = self.pool.begin().await?;
        let queue_id = lock_open_queue(&mut transaction, queue_reference).await?;
        let existing = load_items_in_transaction(&mut transaction, &queue_id).await?;
        let plan = reconcile(&existing, observations)?;
        let now = Utc::now();

        // Release paths from non-current items before attaching moved or new items.
        for action in &plan.actions {
            if let ReconcileAction::Update {
                item_id, freshness, ..
            } = action
                && freshness.state != FreshnessState::Current
            {
                update_freshness(&mut transaction, &queue_id, item_id, None, freshness, now)
                    .await?;
            }
        }

        for action in &plan.actions {
            match action {
                ReconcileAction::Add { observation } => {
                    ensure_current_path_available(
                        &mut transaction,
                        &queue_id,
                        observation.path.as_deref(),
                        None,
                    )
                    .await?;
                    insert_item(&mut transaction, &queue_id, observation, now).await?;
                }
                ReconcileAction::Update {
                    item_id,
                    path,
                    freshness,
                } if freshness.state == FreshnessState::Current => {
                    ensure_current_path_available(
                        &mut transaction,
                        &queue_id,
                        path.as_deref(),
                        Some(item_id),
                    )
                    .await?;
                    update_freshness(
                        &mut transaction,
                        &queue_id,
                        item_id,
                        path.as_deref(),
                        freshness,
                        now,
                    )
                    .await?;
                }
                ReconcileAction::Update { .. } => {}
                ReconcileAction::ReplaceInline {
                    item_id,
                    observation,
                } => {
                    sqlx::query(
                        r#"UPDATE triage_items
                           SET markdown = $1, content_sha256 = $2, source = $3,
                               captured_at = $4, freshness_state = 'current',
                               freshness_reason = NULL, candidate_paths = '[]'::jsonb,
                               freshness_observed_at = $4
                           WHERE queue_id = $5 AND id = $6 AND source_kind = 'inline'"#,
                    )
                    .bind(&observation.markdown)
                    .bind(observation.content_sha256())
                    .bind(&observation.source)
                    .bind(now)
                    .bind(&queue_id)
                    .bind(item_id)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
        }
        sqlx::query("UPDATE triage_queues SET updated_at = $1 WHERE id = $2")
            .bind(now)
            .bind(&queue_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(plan.stats)
    }

    pub async fn attach_item(
        &self,
        queue_reference: &str,
        item_id: &str,
        observation: Observation,
    ) -> Result<TriageItem, TriageStoreError> {
        let mut transaction = self.pool.begin().await?;
        let queue_id = lock_open_queue(&mut transaction, queue_reference).await?;
        let item = load_item_for_update(&mut transaction, &queue_id, item_id).await?;
        let ReconcileAction::Update {
            path, freshness, ..
        } = attachment_action(&item, observation)?
        else {
            return Err(TriageStoreError::Corrupt(
                "attachment produced a non-update action".to_string(),
            ));
        };
        ensure_current_path_available(&mut transaction, &queue_id, path.as_deref(), Some(item_id))
            .await?;
        let now = Utc::now();
        update_freshness(
            &mut transaction,
            &queue_id,
            item_id,
            path.as_deref(),
            &freshness,
            now,
        )
        .await?;
        sqlx::query("UPDATE triage_queues SET updated_at = $1 WHERE id = $2")
            .bind(now)
            .bind(&queue_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.get_item(&queue_id, item_id)
            .await?
            .ok_or_else(|| TriageStoreError::ItemNotFound {
                queue_id,
                item_id: item_id.to_string(),
            })
    }

    pub async fn close_queue(
        &self,
        queue_reference: &str,
    ) -> Result<TriageQueue, TriageStoreError> {
        self.set_queue_status(queue_reference, QueueStatus::Closed)
            .await
    }

    pub async fn reopen_queue(
        &self,
        queue_reference: &str,
    ) -> Result<TriageQueue, TriageStoreError> {
        self.set_queue_status(queue_reference, QueueStatus::Open)
            .await
    }

    pub async fn list_revisions(
        &self,
        queue_reference: &str,
        after_event_id: i64,
        limit: usize,
    ) -> Result<RevisionPage, TriageStoreError> {
        let queue = self
            .get_queue(queue_reference)
            .await?
            .ok_or_else(|| TriageStoreError::QueueNotFound(queue_reference.to_string()))?;
        if after_event_id < 0 {
            return Err(TriageStoreError::Validation(
                "cursor event id must not be negative".to_string(),
            ));
        }
        if limit == 0 || limit > 1000 {
            return Err(TriageStoreError::Validation(
                "revision page limit must be between 1 and 1000".to_string(),
            ));
        }
        let rows = sqlx::query(
            r#"SELECT event_id, queue_id, item_id, revision, verdict, target,
                      content_sha256, decided_at
               FROM triage_revisions
               WHERE queue_id = $1 AND event_id > $2
               ORDER BY event_id
               LIMIT $3"#,
        )
        .bind(&queue.id)
        .bind(after_event_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let items = rows
            .iter()
            .map(row_to_revision)
            .collect::<Result<Vec<_>, _>>()?;
        let next_event_id = items
            .last()
            .map(|revision| revision.event_id)
            .unwrap_or(after_event_id);
        Ok(RevisionPage {
            items,
            next_event_id,
        })
    }

    async fn set_queue_status(
        &self,
        queue_reference: &str,
        status: QueueStatus,
    ) -> Result<TriageQueue, TriageStoreError> {
        let mut transaction = self.pool.begin().await?;
        let queue_id = lock_queue(&mut transaction, queue_reference).await?;
        let now = Utc::now();
        sqlx::query(
            "UPDATE triage_queues SET status = $1, closed_at = $2, updated_at = $3 WHERE id = $4",
        )
        .bind(status.as_str())
        .bind((status == QueueStatus::Closed).then_some(now))
        .bind(now)
        .bind(&queue_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_queue(&queue_id)
            .await?
            .ok_or_else(|| TriageStoreError::QueueNotFound(queue_id))
    }

    async fn load_items(&self, queue_id: &str) -> Result<Vec<TriageItem>, TriageStoreError> {
        let query = format!(
            "{} WHERE i.queue_id = $1 ORDER BY CASE i.source_kind WHEN 'path' THEN 0 ELSE 1 END, i.source_path NULLS LAST, i.id",
            ITEM_DETAIL_SQL
        );
        let rows = sqlx::query(&query)
            .bind(queue_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_item).collect()
    }

    async fn get_item(
        &self,
        queue_id: &str,
        item_id: &str,
    ) -> Result<Option<TriageItem>, TriageStoreError> {
        let query = format!("{} WHERE i.queue_id = $1 AND i.id = $2", ITEM_DETAIL_SQL);
        let row = sqlx::query(&query)
            .bind(queue_id)
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_item).transpose()
    }
}

fn validate_create_request(request: &CreateQueue) -> Result<(), TriageStoreError> {
    if request.name.is_empty() || request.name.trim() != request.name {
        return Err(TriageStoreError::Validation(
            "queue name must be non-empty and have no surrounding whitespace".to_string(),
        ));
    }
    if request.root.is_empty() || !std::path::Path::new(&request.root).is_absolute() {
        return Err(TriageStoreError::Validation(
            "queue root must be an absolute path".to_string(),
        ));
    }
    if request.strip_suffix.is_empty() {
        return Err(TriageStoreError::Validation(
            "strip suffix must not be empty".to_string(),
        ));
    }
    validate_persisted_source_spec(&request.source_spec)?;
    Ok(())
}

fn validate_target_text(target: String) -> Result<String, TriageStoreError> {
    if target.is_empty() || target.trim() != target {
        return Err(TriageStoreError::Validation(
            "merge target must be non-empty and have no surrounding whitespace".to_string(),
        ));
    }
    Ok(target)
}

async fn validate_decision(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    item_id: &str,
    verdict: Option<TriageVerdict>,
    target: Option<&str>,
) -> Result<(), TriageStoreError> {
    match verdict {
        Some(TriageVerdict::MergeInto) => {
            let target = target.ok_or_else(|| {
                TriageStoreError::Validation("merge_into requires a target".to_string())
            })?;
            if target == item_id {
                return Err(TriageStoreError::Validation(
                    "merge_into target must be a different item".to_string(),
                ));
            }
            let target_exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM triage_items WHERE queue_id = $1 AND id = $2)",
            )
            .bind(queue_id)
            .bind(target)
            .fetch_one(&mut **transaction)
            .await?;
            if !target_exists {
                return Err(TriageStoreError::Validation(format!(
                    "merge target is not in this queue: {target}"
                )));
            }
            let reciprocal = sqlx::query_scalar::<_, bool>(
                r#"SELECT EXISTS(
                       SELECT 1
                       FROM triage_revisions r
                       WHERE r.queue_id = $1 AND r.item_id = $2
                         AND r.revision = (
                             SELECT MAX(latest.revision)
                             FROM triage_revisions latest
                             WHERE latest.queue_id = r.queue_id AND latest.item_id = r.item_id
                         )
                         AND r.verdict = 'merge_into' AND r.target = $3
                   )"#,
            )
            .bind(queue_id)
            .bind(target)
            .bind(item_id)
            .fetch_one(&mut **transaction)
            .await?;
            if reciprocal {
                return Err(TriageStoreError::Validation(format!(
                    "direct reciprocal merge edge is not allowed: {item_id} <-> {target}"
                )));
            }
        }
        _ if target.is_some() => {
            return Err(TriageStoreError::Validation(
                "only merge_into can carry a target".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

async fn insert_item(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    observation: &Observation,
    now: DateTime<Utc>,
) -> Result<(), TriageStoreError> {
    sqlx::query(
        r#"INSERT INTO triage_items
               (queue_id, id, source_kind, source_path, source, markdown,
                content_sha256, captured_at, freshness_state, freshness_reason,
                candidate_paths, freshness_observed_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'current', NULL, '[]'::jsonb, $8)"#,
    )
    .bind(queue_id)
    .bind(&observation.id)
    .bind(observation.source_kind().as_str())
    .bind(&observation.path)
    .bind(&observation.source)
    .bind(&observation.markdown)
    .bind(observation.content_sha256())
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn update_freshness(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    item_id: &str,
    path: Option<&str>,
    freshness: &Freshness,
    now: DateTime<Utc>,
) -> Result<(), TriageStoreError> {
    let candidates = serde_json::to_value(&freshness.candidate_paths)?;
    sqlx::query(
        r#"UPDATE triage_items
           SET source_path = COALESCE($1, source_path), freshness_state = $2,
               freshness_reason = $3, candidate_paths = $4, freshness_observed_at = $5
           WHERE queue_id = $6 AND id = $7"#,
    )
    .bind(path)
    .bind(freshness.state.as_str())
    .bind(&freshness.reason)
    .bind(candidates)
    .bind(now)
    .bind(queue_id)
    .bind(item_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn ensure_current_path_available(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    path: Option<&str>,
    except_item_id: Option<&str>,
) -> Result<(), TriageStoreError> {
    let Some(path) = path else {
        return Ok(());
    };
    let conflict = sqlx::query_scalar::<_, String>(
        r#"SELECT id
           FROM triage_items
           WHERE queue_id = $1 AND source_path = $2 AND freshness_state = 'current'
             AND ($3::TEXT IS NULL OR id <> $3)
           LIMIT 1"#,
    )
    .bind(queue_id)
    .bind(path)
    .bind(except_item_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(conflict) = conflict {
        return Err(TriageStoreError::Validation(format!(
            "path {path} is already attached to item {conflict}"
        )));
    }
    Ok(())
}

async fn lock_queue(
    transaction: &mut Transaction<'_, Postgres>,
    queue_reference: &str,
) -> Result<String, TriageStoreError> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT id
           FROM triage_queues
           WHERE id = $1 OR name = $1
           ORDER BY (id = $1) DESC
           LIMIT 1
           FOR UPDATE"#,
    )
    .bind(queue_reference)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| TriageStoreError::QueueNotFound(queue_reference.to_string()))
}

async fn lock_open_queue(
    transaction: &mut Transaction<'_, Postgres>,
    queue_reference: &str,
) -> Result<String, TriageStoreError> {
    let queue_id = lock_queue(transaction, queue_reference).await?;
    let status = sqlx::query_scalar::<_, String>("SELECT status FROM triage_queues WHERE id = $1")
        .bind(&queue_id)
        .fetch_one(&mut **transaction)
        .await?;
    if status == "closed" {
        return Err(TriageStoreError::ClosedQueue);
    }
    Ok(queue_id)
}

async fn load_item_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    item_id: &str,
) -> Result<TriageItem, TriageStoreError> {
    let query = format!(
        "{} WHERE i.queue_id = $1 AND i.id = $2 FOR UPDATE OF i",
        ITEM_DETAIL_SQL
    );
    let row = sqlx::query(&query)
        .bind(queue_id)
        .bind(item_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(|| TriageStoreError::ItemNotFound {
            queue_id: queue_id.to_string(),
            item_id: item_id.to_string(),
        })?;
    row_to_item(&row)
}

async fn load_items_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
) -> Result<Vec<TriageItem>, TriageStoreError> {
    let query = format!(
        "{} WHERE i.queue_id = $1 ORDER BY CASE i.source_kind WHEN 'path' THEN 0 ELSE 1 END, i.source_path NULLS LAST, i.id FOR UPDATE OF i",
        ITEM_DETAIL_SQL
    );
    let rows = sqlx::query(&query)
        .bind(queue_id)
        .fetch_all(&mut **transaction)
        .await?;
    rows.iter().map(row_to_item).collect()
}

async fn load_latest_revision(
    transaction: &mut Transaction<'_, Postgres>,
    queue_id: &str,
    item_id: &str,
) -> Result<Option<TriageRevision>, TriageStoreError> {
    let row = sqlx::query(
        r#"SELECT event_id, queue_id, item_id, revision, verdict, target,
                  content_sha256, decided_at
           FROM triage_revisions
           WHERE queue_id = $1 AND item_id = $2
           ORDER BY revision DESC
           LIMIT 1"#,
    )
    .bind(queue_id)
    .bind(item_id)
    .fetch_optional(&mut **transaction)
    .await?;
    row.as_ref().map(row_to_revision).transpose()
}

fn row_to_queue(row: &PgRow) -> Result<TriageQueue, TriageStoreError> {
    let total: i64 = row.try_get("total")?;
    let decided: i64 = row.try_get("decided")?;
    Ok(TriageQueue {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        root: row.try_get("root")?,
        strip_suffix: row.try_get("strip_suffix")?,
        source_spec: serde_json::from_value(row.try_get::<Value, _>("source_spec")?)?,
        status: row
            .try_get::<String, _>("status")?
            .parse()
            .map_err(TriageStoreError::Corrupt)?,
        complete: total > 0 && decided == total,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        closed_at: row.try_get("closed_at")?,
        counts: QueueCounts {
            total,
            decided,
            stale: row.try_get("stale")?,
            ambiguous: row.try_get("ambiguous")?,
        },
    })
}

fn row_to_item(row: &PgRow) -> Result<TriageItem, TriageStoreError> {
    let queue_id: String = row.try_get("queue_id")?;
    let id: String = row.try_get("id")?;
    let latest_revision = row
        .try_get::<Option<i64>, _>("event_id")?
        .map(|event_id| {
            let verdict = row
                .try_get::<Option<String>, _>("verdict")?
                .map(|value| value.parse().map_err(TriageStoreError::Corrupt))
                .transpose()?;
            Ok::<TriageRevision, TriageStoreError>(TriageRevision {
                event_id,
                queue_id: queue_id.clone(),
                item_id: id.clone(),
                revision: row.try_get("revision")?,
                verdict,
                target: row.try_get("target")?,
                content_sha256: row.try_get("revision_content_sha256")?,
                decided_at: row.try_get("decided_at")?,
            })
        })
        .transpose()?;
    let candidate_paths = serde_json::from_value(row.try_get::<Value, _>("candidate_paths")?)?;
    Ok(TriageItem {
        queue_id,
        id,
        source_kind: row
            .try_get::<String, _>("source_kind")?
            .parse()
            .map_err(TriageStoreError::Corrupt)?,
        path: row.try_get("source_path")?,
        source: row.try_get("source")?,
        markdown: row.try_get("markdown")?,
        content_sha256: row.try_get("content_sha256")?,
        captured_at: row.try_get("captured_at")?,
        freshness: Freshness {
            state: row
                .try_get::<String, _>("freshness_state")?
                .parse()
                .map_err(TriageStoreError::Corrupt)?,
            reason: row.try_get("freshness_reason")?,
            candidate_paths,
        },
        latest_revision,
    })
}

fn row_to_revision(row: &PgRow) -> Result<TriageRevision, TriageStoreError> {
    let verdict = row
        .try_get::<Option<String>, _>("verdict")?
        .map(|value| value.parse().map_err(TriageStoreError::Corrupt))
        .transpose()?;
    Ok(TriageRevision {
        event_id: row.try_get("event_id")?,
        queue_id: row.try_get("queue_id")?,
        item_id: row.try_get("item_id")?,
        revision: row.try_get("revision")?,
        verdict,
        target: row.try_get("target")?,
        content_sha256: row.try_get("content_sha256")?,
        decided_at: row.try_get("decided_at")?,
    })
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "23505")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn path_item(id: &str, path: &str, markdown: &str) -> Observation {
        Observation {
            id: id.to_string(),
            markdown: markdown.to_string(),
            path: Some(path.to_string()),
            source: None,
        }
    }

    fn inline_item(id: &str, markdown: &str) -> Observation {
        Observation {
            id: id.to_string(),
            markdown: markdown.to_string(),
            path: None,
            source: Some(json!({"fixture": true})),
        }
    }

    fn request(name: &str, items: Vec<Observation>) -> CreateQueue {
        CreateQueue {
            name: name.to_string(),
            root: "/tmp/triage-fixture".to_string(),
            strip_suffix: DEFAULT_STRIP_SUFFIX.to_string(),
            source_spec: SourceSpec::Glob {
                patterns: vec!["**/*.md".to_string()],
            },
            items,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs DATABASE_URL: run against a disposable PostgreSQL database"]
    async fn queue_creation_refresh_and_duplicate_name_are_durable(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let store = TriageStore::new(pool);
        let queue = store
            .create_queue(request(
                "refresh-fixture",
                vec![
                    inline_item("z-inline", "old inline"),
                    path_item("b", "b.md", "b old"),
                    path_item("a", "a.md", "a"),
                    path_item("moved", "old.md", "same bytes"),
                    path_item("missing", "missing.md", "gone"),
                ],
            ))
            .await?;
        let duplicate = store
            .create_queue(request("refresh-fixture", vec![]))
            .await
            .unwrap_err();
        assert!(matches!(
            duplicate,
            TriageStoreError::DuplicateQueue { existing_id, .. } if existing_id == queue.id
        ));

        let before = store.get_detail(&queue.id).await?.unwrap();
        assert_eq!(
            before
                .catalog
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "missing", "moved", "z-inline"]
        );
        let b_snapshot = before
            .catalog
            .iter()
            .find(|item| item.id == "b")
            .unwrap()
            .markdown
            .clone();

        let stats = store
            .refresh_queue(
                &queue.id,
                vec![
                    path_item("a", "a.md", "a"),
                    path_item("b", "b.md", "b changed"),
                    path_item("different-declared-id", "new.md", "same bytes"),
                    path_item("new", "brand-new.md", "new"),
                    inline_item("z-inline", "new inline"),
                ],
            )
            .await?;
        assert_eq!(stats.content_changed, 1);
        assert_eq!(stats.moved, 1);
        assert_eq!(stats.missing, 1);
        assert_eq!(stats.added, 1);
        assert_eq!(stats.inline_replaced, 1);

        let after = store.get_detail(&queue.id).await?.unwrap();
        let b = after.catalog.iter().find(|item| item.id == "b").unwrap();
        assert_eq!(b.markdown, b_snapshot);
        assert_eq!(b.freshness.state, FreshnessState::ContentChanged);
        let moved = after
            .catalog
            .iter()
            .find(|item| item.id == "moved")
            .unwrap();
        assert_eq!(moved.path.as_deref(), Some("new.md"));
        assert_eq!(moved.markdown, "same bytes");
        let inline = after
            .catalog
            .iter()
            .find(|item| item.id == "z-inline")
            .unwrap();
        assert_eq!(inline.markdown, "new inline");

        let ambiguity = store
            .refresh_queue(
                &queue.id,
                vec![
                    path_item("a", "a.md", "a"),
                    path_item("b", "b.md", "b changed"),
                    path_item("moved", "new.md", "same bytes"),
                    path_item("new", "brand-new.md", "new"),
                    path_item("candidate-one", "candidate-one.md", "gone"),
                    path_item("candidate-two", "candidate-two.md", "gone"),
                    inline_item("z-inline", "new inline"),
                ],
            )
            .await?;
        assert_eq!(ambiguity.ambiguous, 1);
        let ambiguous = store
            .get_detail(&queue.id)
            .await?
            .unwrap()
            .catalog
            .into_iter()
            .find(|item| item.id == "missing")
            .unwrap();
        assert_eq!(ambiguous.freshness.state, FreshnessState::Ambiguous);
        assert_eq!(
            ambiguous.freshness.candidate_paths,
            vec!["candidate-one.md", "candidate-two.md"]
        );
        let attached = store
            .attach_item(
                &queue.id,
                "missing",
                path_item("candidate-one", "candidate-one.md", "gone"),
            )
            .await?;
        assert_eq!(attached.path.as_deref(), Some("candidate-one.md"));
        assert_eq!(attached.markdown, "gone");
        assert_eq!(attached.freshness.state, FreshnessState::Current);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs DATABASE_URL: run against a disposable PostgreSQL database"]
    async fn decisions_are_transactional_append_only_and_sequence_paginated(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let store = TriageStore::new(pool);
        let queue = store
            .create_queue(request(
                "decision-fixture",
                vec![
                    inline_item("a", "A v1"),
                    inline_item("b", "B"),
                    inline_item("c", "C"),
                    path_item("path", "path.md", "immutable path snapshot"),
                ],
            ))
            .await?;

        for verdict in [
            TriageVerdict::Schedule,
            TriageVerdict::Delete,
            TriageVerdict::NeedsReplan,
            TriageVerdict::LeaveCaptured,
        ] {
            store
                .record_decision(
                    &queue.id,
                    "a",
                    DecisionPatch {
                        verdict: PatchField::Value(verdict),
                        target: PatchField::Missing,
                    },
                )
                .await?;
        }
        let merge = store
            .record_decision(
                &queue.id,
                "a",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::MergeInto),
                    target: PatchField::Value("b".to_string()),
                },
            )
            .await?;
        assert_eq!(merge.revision, 5);
        assert_eq!(merge.target.as_deref(), Some("b"));

        let reciprocal = store
            .record_decision(
                &queue.id,
                "b",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::MergeInto),
                    target: PatchField::Value("a".to_string()),
                },
            )
            .await
            .unwrap_err();
        assert!(reciprocal.to_string().contains("reciprocal"));
        let self_target = store
            .record_decision(
                &queue.id,
                "b",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::MergeInto),
                    target: PatchField::Value("b".to_string()),
                },
            )
            .await
            .unwrap_err();
        assert!(self_target.to_string().contains("different item"));

        store
            .record_decision(
                &queue.id,
                "b",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::MergeInto),
                    target: PatchField::Value("c".to_string()),
                },
            )
            .await?;
        store
            .record_decision(
                &queue.id,
                "c",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::MergeInto),
                    target: PatchField::Value("a".to_string()),
                },
            )
            .await?;
        store
            .record_decision(
                &queue.id,
                "path",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::Schedule),
                    target: PatchField::Missing,
                },
            )
            .await?;

        let completed = store.get_queue(&queue.id).await?.unwrap();
        assert_eq!(completed.counts.decided, 4);
        assert!(completed.complete);
        assert_eq!(completed.status, QueueStatus::Open);

        let revision_rewrite =
            sqlx::query("UPDATE triage_revisions SET verdict = 'delete' WHERE event_id = $1")
                .bind(merge.event_id)
                .execute(&store.pool)
                .await
                .unwrap_err();
        assert!(revision_rewrite.to_string().contains("append-only"));
        let snapshot_rewrite = sqlx::query(
            "UPDATE triage_items SET markdown = 'changed' WHERE queue_id = $1 AND id = 'path'",
        )
        .bind(&queue.id)
        .execute(&store.pool)
        .await
        .unwrap_err();
        assert!(snapshot_rewrite.to_string().contains("immutable"));

        let first_page = store.list_revisions(&queue.id, 0, 3).await?;
        assert_eq!(first_page.items.len(), 3);
        let second_page = store
            .list_revisions(&queue.id, first_page.next_event_id, 100)
            .await?;
        assert!(
            second_page
                .items
                .iter()
                .all(|item| item.event_id > first_page.next_event_id)
        );

        let cleared = store
            .record_decision(
                &queue.id,
                "a",
                DecisionPatch {
                    verdict: PatchField::Null,
                    target: PatchField::Missing,
                },
            )
            .await?;
        assert_eq!(cleared.verdict, None);
        assert_eq!(cleared.target, None);
        let retained = store
            .record_decision(&queue.id, "b", DecisionPatch::default())
            .await?;
        assert_eq!(retained.verdict, Some(TriageVerdict::MergeInto));
        assert_eq!(retained.target.as_deref(), Some("c"));

        let first_hash = cleared.content_sha256;
        store
            .refresh_queue(
                &queue.id,
                vec![
                    inline_item("a", "A v2"),
                    inline_item("b", "B"),
                    inline_item("c", "C"),
                ],
            )
            .await?;
        let revised = store
            .record_decision(
                &queue.id,
                "a",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::Schedule),
                    target: PatchField::Missing,
                },
            )
            .await?;
        assert_ne!(first_hash, revised.content_sha256);

        store.close_queue(&queue.id).await?;
        assert!(matches!(
            store
                .record_decision(
                    &queue.id,
                    "a",
                    DecisionPatch {
                        verdict: PatchField::Value(TriageVerdict::Delete),
                        target: PatchField::Missing,
                    },
                )
                .await,
            Err(TriageStoreError::ClosedQueue)
        ));
        assert!(matches!(
            store.refresh_queue(&queue.id, vec![]).await,
            Err(TriageStoreError::ClosedQueue)
        ));
        store.reopen_queue(&queue.id).await?;
        store
            .record_decision(
                &queue.id,
                "a",
                DecisionPatch {
                    verdict: PatchField::Value(TriageVerdict::Delete),
                    target: PatchField::Missing,
                },
            )
            .await?;
        Ok(())
    }
}
