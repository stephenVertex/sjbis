use crate::grants::{
    GrantError, GrantRecord, StoredGrant, check_grant_for_notification, grant_answer,
};
use crate::models::*;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

// Explicit column list to avoid PostgreSQL "cached plan must not change result type"
// errors when columns are added via migrations.
const NOTIF_COLS: &str = "id, agent_name, instance, sender, src, question, detail, detail_markdown, question_type, urgency, blocking, deadline, reply_to, status, created_at, answered_at, answer, answer_label, choices, yes_label, no_label, placeholder, suggestions, min, max, step, default_value, unit, accept, diff, ack_label, items, slots, mute_key, caller_id, snooze_until, note, sub_questions, ttl_secs, renews, once";

const GRANT_COLS: &str = "id, kind, notification_id, record, record_sha256, sig, signer, issued_at, expires_at, renews, revoked_by, claimed_by, claimed_at, created_at";

impl Db {
    #[cfg(test)]
    pub(crate) fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(dsn: &str) -> Result<Self> {
        let pool = PgPool::connect(dsn).await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await?;
        Ok(Self { pool })
    }

    #[allow(dead_code)]
    pub fn triage(&self) -> crate::triage::store::TriageStore {
        crate::triage::store::TriageStore::new(self.pool.clone())
    }

    // ── Notifications ────────────────────────────────────────────────────

    pub async fn insert_notification(&self, n: &Notification) -> Result<()> {
        let choices = n.choices.as_ref().map(|c| serde_json::to_value(c).unwrap());
        let suggestions = n.suggestions.as_ref().map(|s| serde_json::to_value(s).unwrap());
        let diff = n.diff.as_ref().map(|d| serde_json::to_value(d).unwrap());
        let items = n.items.as_ref().map(|i| serde_json::to_value(i).unwrap());
        let slots = n.slots.as_ref().map(|s| serde_json::to_value(s).unwrap());
        let sub_questions = n.sub_questions.as_ref().map(|s| serde_json::to_value(s).unwrap());
        let deadline = n.deadline;
        let created_at = n.created_at;
        let answered_at = n.answered_at;
        let reply_to = serde_json::to_value(&n.reply_to)?;

        sqlx::query(
            r#"INSERT INTO notifications (
                id, agent_name, instance, sender, src, question, detail, detail_markdown,
                question_type, urgency, blocking, deadline, reply_to, status,
                created_at, answered_at, answer, answer_label,
                choices, yes_label, no_label, placeholder, suggestions,
                min, max, step, default_value, unit, accept, diff, ack_label,
                items, slots, mute_key, caller_id, sub_questions, ttl_secs, renews, once
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                      $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26,
                      $27, $28, $29, $30, $31, $32, $33, $34, $35, $36, $37, $38, $39)"#,
        )
        .bind(&n.id)
        .bind(&n.agent_name)
        .bind(&n.instance)
        .bind(&n.sender)
        .bind(&n.src)
        .bind(&n.question)
        .bind(&n.detail)
        .bind(&n.detail_markdown)
        .bind(n.question_type.to_string())
        .bind(n.urgency)
        .bind(n.blocking)
        .bind(deadline)
        .bind(reply_to)
        .bind(format!("{:?}", n.status).to_lowercase())
        .bind(created_at)
        .bind(answered_at)
        .bind(&n.answer)
        .bind(&n.answer_label)
        .bind(choices)
        .bind(&n.yes_label)
        .bind(&n.no_label)
        .bind(&n.placeholder)
        .bind(suggestions)
        .bind(n.min)
        .bind(n.max)
        .bind(n.step)
        .bind(n.default_value)
        .bind(&n.unit)
        .bind(&n.accept)
        .bind(diff)
        .bind(&n.ack_label)
        .bind(items)
        .bind(slots)
        .bind(&n.mute_key)
        .bind(&n.caller_id)
        .bind(sub_questions)
        .bind(n.ttl_secs)
        .bind(&n.renews)
        .bind(n.once)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_notification(&self, id: &str) -> Result<Option<Notification>> {
        let row = sqlx::query(&format!("SELECT {} FROM notifications WHERE id = $1", NOTIF_COLS))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => Ok(Some(Self::row_to_notification(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn get_notification_by_caller_id(
        &self,
        caller_id: &str,
        since: DateTime<Utc>,
    ) -> Result<Option<Notification>> {
        let row = sqlx::query(
            &format!("SELECT {} FROM notifications WHERE caller_id = $1 AND created_at > $2 ORDER BY created_at DESC LIMIT 1", NOTIF_COLS),
        )
        .bind(caller_id)
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(Some(Self::row_to_notification(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn list_open_notifications(&self) -> Result<Vec<Notification>> {
        let now = Utc::now();
        let rows = sqlx::query(
            &format!("SELECT {} FROM notifications WHERE status = 'open' AND (snooze_until IS NULL OR snooze_until <= $1) AND (deadline IS NULL OR deadline > $1) ORDER BY urgency DESC, created_at DESC", NOTIF_COLS),
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Self::row_to_notification(r))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn snooze_notification(&self, id: &str, minutes: i64) -> Result<Option<Notification>> {
        let now = Utc::now();
        let row = sqlx::query(
            &format!("UPDATE notifications SET snooze_until = LEAST($1 + ($2 * INTERVAL '1 minute'), COALESCE(deadline, $1 + ($2 * INTERVAL '1 minute'))) WHERE id = $3 RETURNING {}", NOTIF_COLS)
        )
        .bind(now)
        .bind(minutes)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(Some(Self::row_to_notification(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn list_history(&self, limit: usize) -> Result<Vec<Notification>> {
        let rows = sqlx::query(
            &format!("SELECT {} FROM notifications WHERE status IN ('answered', 'timed_out') ORDER BY answered_at DESC, created_at DESC LIMIT $1", NOTIF_COLS),
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Self::row_to_notification(r))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn update_status(&self, id: &str, status: NotificationStatus) -> Result<()> {
        sqlx::query("UPDATE notifications SET status = $1 WHERE id = $2")
            .bind(format!("{:?}", status).to_lowercase())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn answer_notification(
        &self,
        id: &str,
        answer: &str,
        answer_label: Option<&str>,
        note: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE notifications SET status = 'answered', answer = $1, answer_label = $2, answered_at = $3, note = $4 WHERE id = $5",
        )
        .bind(answer)
        .bind(answer_label)
        .bind(now)
        .bind(note)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn dismiss_notification(&self, id: &str) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE notifications SET status = 'dismissed', answered_at = $1 WHERE id = $2",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_notification(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM notifications WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn timeout_notifications(&self) -> Result<Vec<Notification>> {
        let now = Utc::now();
        let rows = sqlx::query(
            &format!("SELECT {} FROM notifications WHERE status = 'open' AND deadline IS NOT NULL AND deadline < $1", NOTIF_COLS),
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let timed_out: Vec<Notification> = rows
            .iter()
            .map(|r| Self::row_to_notification(r))
            .collect::<Result<Vec<_>>>()?;

        sqlx::query(
            "UPDATE notifications SET status = 'timed_out' WHERE status = 'open' AND deadline IS NOT NULL AND deadline < $1",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(timed_out)
    }

    // ── Rules ──────────────────────────────────────────────────────────

    pub async fn insert_rule(&self, rule: &Rule) -> Result<()> {
        let active_window = rule
            .active_window
            .as_ref()
            .map(|w| serde_json::to_value(w).unwrap());
        let created_at = rule.created_at;
        let expires_at = rule.expires_at;

        sqlx::query(
            r#"INSERT INTO rules (
                id, text, compiled, active, scope, urgency_min, mute, priority,
                expires_at, active_window, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(&rule.id)
        .bind(&rule.text)
        .bind(rule.compiled.clone())
        .bind(rule.active)
        .bind(&rule.scope)
        .bind(rule.urgency_min)
        .bind(rule.mute)
        .bind(rule.priority)
        .bind(expires_at)
        .bind(active_window)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_rules(&self) -> Result<Vec<Rule>> {
        let rows = sqlx::query("SELECT * FROM rules ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| Self::row_to_rule(r))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn delete_rule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM rules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Agents ─────────────────────────────────────────────────────────

    pub async fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO agents (name, glyph, color, kind)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (name) DO UPDATE SET
                   glyph = EXCLUDED.glyph,
                   color = EXCLUDED.color,
                   kind = EXCLUDED.kind"#,
        )
        .bind(&agent.name)
        .bind(&agent.glyph)
        .bind(&agent.color)
        .bind(&agent.kind)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_agents(&self) -> Result<Vec<Agent>> {
        let rows = sqlx::query("SELECT * FROM agents")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| Self::row_to_agent(r))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn list_source_summaries(&self) -> Result<Vec<SourceSummary>> {
        let rows = sqlx::query(
            r#"SELECT
                   a.name,
                   a.glyph,
                   a.color,
                   a.kind,
                   MAX(n.created_at) AS last_activity_at,
                   COALESCE(BOOL_OR(n.status = 'open'), FALSE) AS has_open_notification
               FROM agents a
               LEFT JOIN notifications n ON n.agent_name = a.name
               GROUP BY a.name, a.glyph, a.color, a.kind
               ORDER BY a.name"#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Self::row_to_source_summary(r))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn get_or_create_agent(&self, name: &str) -> Result<Agent> {
        if let Some(row) = sqlx::query("SELECT * FROM agents WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
        {
            return Ok(Self::row_to_agent(&row)?);
        }
        let agent = Agent {
            name: name.to_string(),
            glyph: "◐".to_string(),
            color: agent_color(name),
            kind: "unknown".to_string(),
        };
        self.upsert_agent(&agent).await?;
        Ok(agent)
    }

    // ── Device Tokens ──────────────────────────────────────────────────

    pub async fn register_device_token(&self, token: &str, device_name: Option<&str>) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO device_tokens (token, device_name)
               VALUES ($1, $2)
               ON CONFLICT (token) DO UPDATE SET
                   device_name = EXCLUDED.device_name,
                   last_seen = NOW()"#,
        )
        .bind(token)
        .bind(device_name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn unregister_device_token(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM device_tokens WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_device_tokens(&self) -> Result<Vec<(String, Option<String>)>> {
        let rows = sqlx::query("SELECT token, device_name FROM device_tokens")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| Ok((r.try_get::<String, _>("token")?, r.try_get::<Option<String>, _>("device_name")?)))
            .collect()
    }

    // ── Phone devices / Web Push (/phone) ───────────────────────────────

    fn push_device_row(r: &sqlx::postgres::PgRow) -> Result<crate::push_handlers::Device> {
        let endpoint: Option<String> = r.try_get("endpoint")?;
        let p256dh: Option<String> = r.try_get("p256dh")?;
        let auth: Option<String> = r.try_get("auth")?;
        let subscription = match (endpoint, p256dh, auth) {
            (Some(endpoint), Some(p256dh), Some(auth)) => Some(crate::webpush::Subscription { endpoint, p256dh, auth }),
            _ => None,
        };
        Ok(crate::push_handlers::Device {
            device_id: r.try_get("device_id")?,
            subscription,
            device_name: r.try_get("device_name")?,
            signer_id: r.try_get("signer_id")?,
            pubkey: r.try_get("pubkey")?,
            user_agent: r.try_get("user_agent")?,
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
            last_push_at: r.try_get("last_push_at")?,
            last_error: r.try_get("last_error")?,
        })
    }

    /// Insert or update a phone device. Fields passed as `None` keep their
    /// stored value, so a page can register its key before it has a push
    /// subscription and add the subscription later.
    pub async fn upsert_push_device(
        &self,
        device_id: &str,
        sub: Option<&crate::webpush::Subscription>,
        device_name: Option<&str>,
        signer_id: Option<&str>,
        pubkey: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<crate::push_handlers::Device> {
        let row = sqlx::query(
            r#"INSERT INTO push_subscriptions (device_id, endpoint, p256dh, auth, device_name, signer_id, pubkey, user_agent)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (device_id) DO UPDATE SET
                 endpoint = COALESCE(EXCLUDED.endpoint, push_subscriptions.endpoint),
                 p256dh = COALESCE(EXCLUDED.p256dh, push_subscriptions.p256dh),
                 auth = COALESCE(EXCLUDED.auth, push_subscriptions.auth),
                 device_name = COALESCE(EXCLUDED.device_name, push_subscriptions.device_name),
                 signer_id = COALESCE(EXCLUDED.signer_id, push_subscriptions.signer_id),
                 pubkey = COALESCE(EXCLUDED.pubkey, push_subscriptions.pubkey),
                 user_agent = COALESCE(EXCLUDED.user_agent, push_subscriptions.user_agent),
                 last_error = CASE WHEN EXCLUDED.endpoint IS NOT NULL THEN NULL ELSE push_subscriptions.last_error END,
                 updated_at = now()
               RETURNING *"#,
        )
        .bind(device_id)
        .bind(sub.map(|s| s.endpoint.as_str()))
        .bind(sub.map(|s| s.p256dh.as_str()))
        .bind(sub.map(|s| s.auth.as_str()))
        .bind(device_name)
        .bind(signer_id)
        .bind(pubkey)
        .bind(user_agent)
        .fetch_one(&self.pool)
        .await?;
        Self::push_device_row(&row)
    }

    pub async fn delete_push_device(&self, device_id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM push_subscriptions WHERE device_id = $1").bind(device_id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn list_push_devices(&self) -> Result<Vec<crate::push_handlers::Device>> {
        let rows = sqlx::query("SELECT * FROM push_subscriptions ORDER BY created_at").fetch_all(&self.pool).await?;
        rows.iter().map(Self::push_device_row).collect()
    }

    pub async fn mark_push_ok(&self, device_id: &str) -> Result<()> {
        sqlx::query("UPDATE push_subscriptions SET last_push_at = now(), last_error = NULL WHERE device_id = $1")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_push_error(&self, device_id: &str, err: &str) -> Result<()> {
        sqlx::query("UPDATE push_subscriptions SET last_error = $2 WHERE device_id = $1")
            .bind(device_id)
            .bind(err)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Drop a dead subscription but keep the device (and its signing key).
    pub async fn clear_push_subscription(&self, device_id: &str, why: &str) -> Result<()> {
        sqlx::query("UPDATE push_subscriptions SET endpoint = NULL, p256dh = NULL, auth = NULL, last_error = $2, updated_at = now() WHERE device_id = $1")
            .bind(device_id)
            .bind(why)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Grants (ta) ─────────────────────────────────────────────────────

    async fn insert_grant_row(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        g: &StoredGrant,
    ) -> Result<bool> {
        let inserted = sqlx::query(&format!(
            "INSERT INTO grants ({GRANT_COLS}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             ON CONFLICT (id) DO NOTHING"
        ))
        .bind(&g.id)
        .bind(&g.kind)
        .bind(&g.notification_id)
        .bind(&g.record)
        .bind(&g.record_sha256)
        .bind(&g.sig)
        .bind(&g.signer)
        .bind(g.issued_at)
        .bind(g.expires_at)
        .bind(&g.renews)
        .bind(&g.revoked_by)
        .bind(&g.claimed_by)
        .bind(g.claimed_at)
        .bind(g.created_at)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        Ok(inserted == 1)
    }

    /// Store a kind-grant envelope and answer its approval notification in one
    /// transaction. Returns the notification as answered, or why the grant was
    /// refused (unknown / not approval / not open / question mismatch / id
    /// collision); nothing is written on refusal.
    pub async fn insert_grant_answering(
        &self,
        g: &StoredGrant,
        record: &GrantRecord,
        answered_at: DateTime<Utc>,
    ) -> Result<std::result::Result<Notification, GrantError>> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(&format!(
            "SELECT {NOTIF_COLS} FROM notifications WHERE id = $1 FOR UPDATE"
        ))
        .bind(&record.ask_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(Err(GrantError::NotificationNotFound(record.ask_id.clone())));
        };
        let mut notif = Self::row_to_notification(&row)?;
        if let Err(e) = check_grant_for_notification(record, &notif) {
            return Ok(Err(e));
        }
        if !Self::insert_grant_row(&mut tx, g).await? {
            return Ok(Err(GrantError::IdCollision(g.id.clone())));
        }
        let a = grant_answer(record);
        sqlx::query(
            "UPDATE notifications SET status = 'answered', answer = $1, answer_label = $2, answered_at = $3, note = $4
             WHERE id = $5 AND status = 'open'",
        )
        .bind(&a.answer)
        .bind(&a.answer_label)
        .bind(answered_at)
        .bind(&a.note)
        .bind(&notif.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        notif.status = NotificationStatus::Answered;
        notif.answer = Some(a.answer);
        notif.answer_label = Some(a.answer_label);
        notif.answered_at = Some(answered_at);
        notif.note = a.note;
        Ok(Ok(notif))
    }

    /// Store a revoke envelope and mark its target revoked (the first revoke
    /// wins; later ones are stored but leave `revoked_by` unchanged).
    pub async fn insert_grant_revoking(
        &self,
        g: &StoredGrant,
        target: &str,
    ) -> Result<std::result::Result<(), GrantError>> {
        let mut tx = self.pool.begin().await?;
        let exists = sqlx::query("SELECT id FROM grants WHERE id = $1 FOR UPDATE")
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !exists {
            return Ok(Err(GrantError::TargetNotFound(target.to_string())));
        }
        if !Self::insert_grant_row(&mut tx, g).await? {
            return Ok(Err(GrantError::IdCollision(g.id.clone())));
        }
        sqlx::query("UPDATE grants SET revoked_by = COALESCE(revoked_by, $1) WHERE id = $2")
            .bind(&g.id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Ok(()))
    }

    /// Store an envelope with no side effects (enroll).
    pub async fn insert_grant_only(
        &self,
        g: &StoredGrant,
    ) -> Result<std::result::Result<(), GrantError>> {
        let mut tx = self.pool.begin().await?;
        if !Self::insert_grant_row(&mut tx, g).await? {
            return Ok(Err(GrantError::IdCollision(g.id.clone())));
        }
        tx.commit().await?;
        Ok(Ok(()))
    }

    pub async fn get_grant(&self, id: &str) -> Result<Option<StoredGrant>> {
        let row = sqlx::query(&format!("SELECT {GRANT_COLS} FROM grants WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(Self::row_to_grant).transpose()
    }

    /// Grants answering notification `ask_id`, oldest first.
    pub async fn list_grants_by_notification(&self, ask_id: &str) -> Result<Vec<StoredGrant>> {
        let rows = sqlx::query(&format!(
            "SELECT {GRANT_COLS} FROM grants WHERE notification_id = $1 ORDER BY issued_at, created_at"
        ))
        .bind(ask_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_grant).collect()
    }

    /// Grants issued at or after `since`, newest first, optionally only those
    /// whose signed record names `agent` as the asker (`ta log`).
    pub async fn list_grants_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        agent: Option<&str>,
        limit: u32,
    ) -> Result<Vec<StoredGrant>> {
        let rows = sqlx::query(&format!(
            "SELECT {GRANT_COLS} FROM grants WHERE issued_at >= $1 AND ($2::text IS NULL OR record->>'asked_by' = $2) \
             ORDER BY issued_at DESC, created_at DESC LIMIT $3"
        ))
        .bind(since)
        .bind(agent)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_grant).collect()
    }

    /// Grants whose signed record renews `id` (reauth children), oldest first.
    pub async fn list_grants_by_renews(&self, id: &str) -> Result<Vec<StoredGrant>> {
        let rows = sqlx::query(&format!(
            "SELECT {GRANT_COLS} FROM grants WHERE renews = $1 ORDER BY issued_at, created_at"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_grant).collect()
    }

    /// Mark a grant claimed if it is not already. Returns false if it was.
    pub async fn claim_grant(&self, id: &str, by: &str, at: DateTime<Utc>) -> Result<bool> {
        let n = sqlx::query(
            "UPDATE grants SET claimed_by = $1, claimed_at = $2 WHERE id = $3 AND claimed_by IS NULL",
        )
        .bind(by)
        .bind(at)
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n == 1)
    }

    fn row_to_grant(row: &sqlx::postgres::PgRow) -> anyhow::Result<StoredGrant> {
        Ok(StoredGrant {
            id: row.try_get("id")?,
            kind: row.try_get("kind")?,
            notification_id: row.try_get("notification_id")?,
            record: row.try_get("record")?,
            record_sha256: row.try_get("record_sha256")?,
            sig: row.try_get("sig")?,
            signer: row.try_get("signer")?,
            issued_at: row.try_get("issued_at")?,
            expires_at: row.try_get("expires_at")?,
            renews: row.try_get("renews")?,
            revoked_by: row.try_get("revoked_by")?,
            claimed_by: row.try_get("claimed_by")?,
            claimed_at: row.try_get("claimed_at")?,
            created_at: row.try_get("created_at")?,
        })
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn row_to_notification(row: &sqlx::postgres::PgRow) -> anyhow::Result<Notification> {
        let question_type: String = row.try_get("question_type")?;
        let status_str: String = row.try_get("status")?;
        let reply_to_val: serde_json::Value = row.try_get("reply_to")?;
        let reply_to: ReplyTo = serde_json::from_value(reply_to_val).unwrap_or_default();

        let choices_val: Option<serde_json::Value> = row.try_get("choices")?;
        let choices = choices_val.and_then(|v| serde_json::from_value(v).ok());

        let suggestions_val: Option<serde_json::Value> = row.try_get("suggestions")?;
        let suggestions = suggestions_val.and_then(|v| serde_json::from_value(v).ok());

        let diff_val: Option<serde_json::Value> = row.try_get("diff")?;
        let diff = diff_val.and_then(|v| serde_json::from_value(v).ok());

        let items_val: Option<serde_json::Value> = row.try_get("items")?;
        let items = items_val.and_then(|v| serde_json::from_value(v).ok());

        let slots_val: Option<serde_json::Value> = row.try_get("slots")?;
        let slots = slots_val.and_then(|v| serde_json::from_value(v).ok());

        let sub_questions_val: Option<serde_json::Value> = row.try_get("sub_questions")?;
        let sub_questions = sub_questions_val.and_then(|v| serde_json::from_value(v).ok());

        let deadline: Option<DateTime<Utc>> = row.try_get("deadline")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let answered_at: Option<DateTime<Utc>> = row.try_get("answered_at")?;

        Ok(Notification {
            id: row.try_get("id")?,
            agent_name: row.try_get("agent_name")?,
            instance: row.try_get("instance")?,
            sender: row.try_get("sender")?,
            src: row.try_get("src")?,
            question: row.try_get("question")?,
            detail: row.try_get("detail")?,
            detail_markdown: row.try_get("detail_markdown")?,
            question_type: question_type.parse().unwrap_or(QuestionType::Ack),
            urgency: row.try_get("urgency")?,
            blocking: row.try_get("blocking")?,
            deadline,
            reply_to,
            status: match status_str.as_str() {
                "answered" => NotificationStatus::Answered,
                "cancelled" => NotificationStatus::Cancelled,
                "muted" => NotificationStatus::Muted,
                "timed_out" => NotificationStatus::TimedOut,
                "dismissed" => NotificationStatus::Dismissed,
                _ => NotificationStatus::Open,
            },
            created_at,
            answered_at,
            answer: row.try_get("answer")?,
            answer_label: row.try_get("answer_label")?,
            choices,
            yes_label: row.try_get("yes_label")?,
            no_label: row.try_get("no_label")?,
            placeholder: row.try_get("placeholder")?,
            suggestions,
            min: row.try_get("min")?,
            max: row.try_get("max")?,
            step: row.try_get("step")?,
            default_value: row.try_get("default_value")?,
            unit: row.try_get("unit")?,
            accept: row.try_get("accept")?,
            diff,
            ack_label: row.try_get("ack_label")?,
            items,
            slots,
            mute_key: row.try_get("mute_key")?,
            caller_id: row.try_get("caller_id")?,
            snooze_until: row.try_get("snooze_until").ok(),
            note: row.try_get("note").ok(),
            sub_questions,
            ttl_secs: row.try_get::<Option<i64>, _>("ttl_secs").ok().flatten(),
            renews: row.try_get::<Option<String>, _>("renews").ok().flatten(),
            once: row.try_get::<Option<bool>, _>("once").ok().flatten(),
        })
    }

    fn row_to_rule(row: &sqlx::postgres::PgRow) -> anyhow::Result<Rule> {
        let compiled_val: Option<serde_json::Value> = row.try_get("compiled")?;
        let active_window_val: Option<serde_json::Value> = row.try_get("active_window")?;
        let expires_at: Option<DateTime<Utc>> = row.try_get("expires_at")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;

        Ok(Rule {
            id: row.try_get("id")?,
            text: row.try_get("text")?,
            compiled: compiled_val,
            active: row.try_get("active")?,
            scope: row.try_get("scope")?,
            urgency_min: row.try_get("urgency_min")?,
            mute: row.try_get("mute")?,
            priority: row.try_get("priority").unwrap_or(0),
            expires_at,
            active_window: active_window_val.and_then(|v| serde_json::from_value(v).ok()),
            created_at,
        })
    }

    fn row_to_agent(row: &sqlx::postgres::PgRow) -> anyhow::Result<Agent> {
        Ok(Agent {
            name: row.try_get("name")?,
            glyph: row.try_get("glyph")?,
            color: row.try_get("color")?,
            kind: row.try_get("kind")?,
        })
    }

    fn row_to_source_summary(row: &sqlx::postgres::PgRow) -> anyhow::Result<SourceSummary> {
        Ok(SourceSummary {
            name: row.try_get("name")?,
            glyph: row.try_get("glyph")?,
            color: row.try_get("color")?,
            kind: row.try_get("kind")?,
            last_activity_at: row.try_get("last_activity_at")?,
            has_open_notification: row.try_get("has_open_notification")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs DATABASE_URL: the sjbis refinery gate runs offline without Postgres; run with `cargo test -- --ignored` against a dev database"]
    async fn source_summaries_derive_latest_activity_and_open_status(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO agents (name, glyph, color, kind) VALUES
                   ('active', 'A', 'red', 'test'),
                   ('open', 'O', 'blue', 'test'),
                   ('idle', 'I', 'gray', 'test')"#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO notifications
                   (id, agent_name, sender, src, question, question_type, status, created_at)
               VALUES
                   ('active-old', 'active', '', '', 'old', 'ack', 'answered',
                    '2026-09-20T10:00:00Z'),
                   ('active-new', 'active', '', '', 'new', 'ack', 'dismissed',
                    '2026-09-26T12:30:00Z'),
                   ('open-old', 'open', '', '', 'answered', 'ack', 'answered',
                    '2026-09-21T09:00:00Z'),
                   ('open-new', 'open', '', '', 'waiting', 'ack', 'open',
                    '2026-09-25T08:15:00Z')"#,
        )
        .execute(&pool)
        .await?;

        let summaries = Db { pool }.list_source_summaries().await?;
        let by_name: std::collections::HashMap<_, _> = summaries
            .into_iter()
            .map(|summary| (summary.name.clone(), summary))
            .collect();
        assert_eq!(by_name.len(), 3);

        let active = &by_name["active"];
        assert_eq!(
            active.last_activity_at,
            Some("2026-09-26T12:30:00Z".parse::<DateTime<Utc>>()?)
        );
        assert!(!active.has_open_notification);

        let open = &by_name["open"];
        assert_eq!(
            open.last_activity_at,
            Some("2026-09-25T08:15:00Z".parse::<DateTime<Utc>>()?)
        );
        assert!(open.has_open_notification);

        let idle = &by_name["idle"];
        assert_eq!(idle.last_activity_at, None);
        assert!(!idle.has_open_notification);

        Ok(())
    }
}
