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
const NOTIF_COLS: &str = "id, agent_name, instance, sender, src, question, detail, detail_markdown, question_type, urgency, blocking, deadline, reply_to, status, created_at, answered_at, answer, answer_label, choices, yes_label, no_label, placeholder, suggestions, min, max, step, default_value, unit, accept, diff, ack_label, items, slots, mute_key, caller_id, snooze_until, note";

impl Db {
    pub async fn connect(dsn: &str) -> Result<Self> {
        let pool = PgPool::connect(dsn).await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await?;
        Ok(Self { pool })
    }

    // ── Notifications ────────────────────────────────────────────────────

    pub async fn insert_notification(&self, n: &Notification) -> Result<()> {
        let choices = n.choices.as_ref().map(|c| serde_json::to_value(c).unwrap());
        let suggestions = n.suggestions.as_ref().map(|s| serde_json::to_value(s).unwrap());
        let diff = n.diff.as_ref().map(|d| serde_json::to_value(d).unwrap());
        let items = n.items.as_ref().map(|i| serde_json::to_value(i).unwrap());
        let slots = n.slots.as_ref().map(|s| serde_json::to_value(s).unwrap());
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
                items, slots, mute_key, caller_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                      $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26,
                      $27, $28, $29, $30, $31, $32, $33, $34, $35)"#,
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
                id, text, compiled, active, scope, urgency_min, mute,
                expires_at, active_window, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(&rule.id)
        .bind(&rule.text)
        .bind(rule.compiled.clone())
        .bind(rule.active)
        .bind(&rule.scope)
        .bind(rule.urgency_min)
        .bind(rule.mute)
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
}
