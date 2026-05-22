use crate::models::*;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                agent_name TEXT NOT NULL,
                instance TEXT,
                sender TEXT NOT NULL DEFAULT '',
                src TEXT NOT NULL DEFAULT '',
                question TEXT NOT NULL,
                detail TEXT,
                question_type TEXT NOT NULL,
                urgency INTEGER NOT NULL DEFAULT 2,
                blocking INTEGER NOT NULL DEFAULT 0,
                deadline TEXT,
                reply_to TEXT NOT NULL DEFAULT 'stdout',
                status TEXT NOT NULL DEFAULT 'open',
                created_at TEXT NOT NULL,
                answered_at TEXT,
                answer TEXT,
                answer_label TEXT,
                choices TEXT,
                yes_label TEXT,
                no_label TEXT,
                placeholder TEXT,
                suggestions TEXT,
                min REAL,
                max REAL,
                step REAL,
                default_value REAL,
                unit TEXT,
                accept TEXT,
                diff TEXT,
                ack_label TEXT,
                items TEXT,
                slots TEXT,
                mute_key TEXT,
                caller_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_notif_status ON notifications(status);
            CREATE INDEX IF NOT EXISTS idx_notif_agent ON notifications(agent_name);
            CREATE INDEX IF NOT EXISTS idx_notif_created ON notifications(created_at);

            CREATE TABLE IF NOT EXISTS rules (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                compiled TEXT,
                active INTEGER NOT NULL DEFAULT 1,
                scope TEXT,
                urgency_min INTEGER NOT NULL DEFAULT 0,
                mute INTEGER NOT NULL DEFAULT 0,
                expires_at TEXT,
                active_window TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agents (
                name TEXT PRIMARY KEY,
                glyph TEXT NOT NULL,
                color TEXT NOT NULL,
                kind TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    // ── Notifications ────────────────────────────────────────────────────

    pub fn insert_notification(&self, n: &Notification) -> Result<()> {
        let choices = n.choices.as_ref().map(|c| serde_json::to_string(c).unwrap());
        let suggestions = n.suggestions.as_ref().map(|s| serde_json::to_string(s).unwrap());
        let diff = n.diff.as_ref().map(|d| serde_json::to_string(d).unwrap());
        let items = n.items.as_ref().map(|i| serde_json::to_string(i).unwrap());
        let slots = n.slots.as_ref().map(|s| serde_json::to_string(s).unwrap());
        let deadline = n.deadline.map(|d| d.to_rfc3339());
        let created_at = n.created_at.to_rfc3339();
        let answered_at = n.answered_at.map(|d| d.to_rfc3339());
        let reply_to = serde_json::to_string(&n.reply_to)?;

        self.conn.execute(
            r#"INSERT INTO notifications (
                id, agent_name, instance, sender, src, question, detail,
                question_type, urgency, blocking, deadline, reply_to, status,
                created_at, answered_at, answer, answer_label,
                choices, yes_label, no_label, placeholder, suggestions,
                min, max, step, default_value, unit, accept, diff, ack_label,
                items, slots, mute_key, caller_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                      ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                      ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34)"#,
            params![
                n.id, n.agent_name, n.instance, n.sender, n.src, n.question, n.detail,
                n.question_type.to_string(), n.urgency, n.blocking as i32, deadline, reply_to, format!("{:?}", n.status).to_lowercase(),
                created_at, answered_at, n.answer, n.answer_label,
                choices, n.yes_label, n.no_label, n.placeholder, suggestions,
                n.min, n.max, n.step, n.default_value, n.unit, n.accept, diff, n.ack_label,
                items, slots, n.mute_key, n.caller_id
            ],
        )?;
        Ok(())
    }

    pub fn get_notification(&self, id: &str) -> Result<Option<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM notifications WHERE id = ?1"
        )?;
        let row = stmt.query_row(params![id], Self::row_to_notification).optional()?;
        Ok(row)
    }

    pub fn get_notification_by_caller_id(&self, caller_id: &str, since: DateTime<Utc>) -> Result<Option<Notification>> {
        let since_str = since.to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT * FROM notifications WHERE caller_id = ?1 AND created_at > ?2 ORDER BY created_at DESC LIMIT 1"
        )?;
        let row = stmt.query_row(params![caller_id, since_str], Self::row_to_notification).optional()?;
        Ok(row)
    }

    pub fn list_open_notifications(&self) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM notifications WHERE status = 'open' ORDER BY urgency DESC, created_at DESC"
        )?;
        let rows = stmt.query_map([], Self::row_to_notification)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn list_history(&self, limit: usize) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM notifications WHERE status IN ('answered', 'timed_out') ORDER BY answered_at DESC, created_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_notification)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn update_status(&self, id: &str, status: NotificationStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE notifications SET status = ?1 WHERE id = ?2",
            params![format!("{:?}", status).to_lowercase(), id],
        )?;
        Ok(())
    }

    pub fn answer_notification(
        &self,
        id: &str,
        answer: &str,
        answer_label: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE notifications SET status = 'answered', answer = ?1, answer_label = ?2, answered_at = ?3 WHERE id = ?4",
            params![answer, answer_label, now, id],
        )?;
        Ok(())
    }

    pub fn delete_notification(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM notifications WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn timeout_notifications(&self) -> Result<Vec<Notification>> {
        let now = Utc::now().to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT * FROM notifications WHERE status = 'open' AND deadline IS NOT NULL AND deadline < ?1"
        )?;
        let rows = stmt.query_map(params![now], Self::row_to_notification)?;
        let timed_out = rows.collect::<Result<Vec<_>, _>>()?;
        // Update them
        self.conn.execute(
            "UPDATE notifications SET status = 'timed_out' WHERE status = 'open' AND deadline IS NOT NULL AND deadline < ?1",
            params![now],
        )?;
        Ok(timed_out)
    }

    // ── Rules ──────────────────────────────────────────────────────────

    pub fn insert_rule(&self, rule: &Rule) -> Result<()> {
        let compiled = rule.compiled.as_ref().map(|c| c.to_string());
        let expires_at = rule.expires_at.map(|d| d.to_rfc3339());
        let active_window = rule.active_window.as_ref().map(|w| serde_json::to_string(w).unwrap());
        let created_at = rule.created_at.to_rfc3339();

        self.conn.execute(
            r#"INSERT INTO rules (
                id, text, compiled, active, scope, urgency_min, mute,
                expires_at, active_window, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                rule.id, rule.text, compiled, rule.active as i32, rule.scope,
                rule.urgency_min, rule.mute as i32, expires_at, active_window, created_at
            ],
        )?;
        Ok(())
    }

    pub fn list_rules(&self) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM rules ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            let compiled_str: Option<String> = row.get("compiled")?;
            let compiled = compiled_str.and_then(|s| serde_json::from_str(&s).ok());
            let expires_at: Option<String> = row.get("expires_at")?;
            let active_window: Option<String> = row.get("active_window")?;
            let created_at: String = row.get("created_at")?;
            Ok(Rule {
                id: row.get("id")?,
                text: row.get("text")?,
                compiled,
                active: row.get::<_, i32>("active")? != 0,
                scope: row.get("scope")?,
                urgency_min: row.get("urgency_min")?,
                mute: row.get::<_, i32>("mute")? != 0,
                expires_at: expires_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                active_window: active_window.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: DateTime::parse_from_rfc3339(&created_at).unwrap().with_timezone(&Utc),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM rules WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    // ── Agents ─────────────────────────────────────────────────────────

    pub fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO agents (name, glyph, color, kind) VALUES (?1, ?2, ?3, ?4)",
            params![agent.name, agent.glyph, agent.color, agent.kind],
        )?;
        Ok(())
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.conn.prepare("SELECT * FROM agents")?;
        let rows = stmt.query_map([], |row| {
            Ok(Agent {
                name: row.get("name")?,
                glyph: row.get("glyph")?,
                color: row.get("color")?,
                kind: row.get("kind")?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn get_or_create_agent(&self, name: &str) -> Result<Agent> {
        if let Some(agent) = {
            let mut stmt = self.conn.prepare("SELECT * FROM agents WHERE name = ?1")?;
            stmt.query_row(params![name], |row| {
                Ok(Agent {
                    name: row.get("name")?,
                    glyph: row.get("glyph")?,
                    color: row.get("color")?,
                    kind: row.get("kind")?,
                })
            }).optional()?
        } {
            return Ok(agent);
        }
        // Create default
        let agent = Agent {
            name: name.to_string(),
            glyph: "◐".to_string(),
            color: agent_color(name),
            kind: "unknown".to_string(),
        };
        self.upsert_agent(&agent)?;
        Ok(agent)
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn row_to_notification(row: &rusqlite::Row) -> rusqlite::Result<Notification> {
        let question_type: String = row.get("question_type")?;
        let reply_to_str: String = row.get("reply_to")?;
        let reply_to: ReplyTo = serde_json::from_str(&reply_to_str).unwrap_or_default();
        let status_str: String = row.get("status")?;
        let status = match status_str.as_str() {
            "answered" => NotificationStatus::Answered,
            "cancelled" => NotificationStatus::Cancelled,
            "muted" => NotificationStatus::Muted,
            "timed_out" => NotificationStatus::TimedOut,
            _ => NotificationStatus::Open,
        };
        let choices: Option<Vec<Choice>> = row.get::<_, Option<String>>("choices")?.and_then(|s| serde_json::from_str(&s).ok());
        let suggestions: Option<Vec<String>> = row.get::<_, Option<String>>("suggestions")?.and_then(|s| serde_json::from_str(&s).ok());
        let diff: Option<Vec<DiffLine>> = row.get::<_, Option<String>>("diff")?.and_then(|s| serde_json::from_str(&s).ok());
        let items: Option<Vec<PickItem>> = row.get::<_, Option<String>>("items")?.and_then(|s| serde_json::from_str(&s).ok());
        let slots: Option<Vec<Slot>> = row.get::<_, Option<String>>("slots")?.and_then(|s| serde_json::from_str(&s).ok());
        let deadline: Option<DateTime<Utc>> = row.get::<_, Option<String>>("deadline")?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc)));
        let created_at: DateTime<Utc> = row.get::<_, String>("created_at")?.parse::<DateTime<chrono::FixedOffset>>().unwrap().with_timezone(&Utc);
        let answered_at: Option<DateTime<Utc>> = row.get::<_, Option<String>>("answered_at")?.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc)));

        Ok(Notification {
            id: row.get("id")?,
            agent_name: row.get("agent_name")?,
            instance: row.get("instance")?,
            sender: row.get("sender")?,
            src: row.get("src")?,
            question: row.get("question")?,
            detail: row.get("detail")?,
            question_type: question_type.parse().unwrap_or(QuestionType::Ack),
            urgency: row.get("urgency")?,
            blocking: row.get::<_, i32>("blocking")? != 0,
            deadline,
            reply_to,
            status,
            created_at,
            answered_at,
            answer: row.get("answer")?,
            answer_label: row.get("answer_label")?,
            choices,
            yes_label: row.get("yes_label")?,
            no_label: row.get("no_label")?,
            placeholder: row.get("placeholder")?,
            suggestions,
            min: row.get("min")?,
            max: row.get("max")?,
            step: row.get("step")?,
            default_value: row.get("default_value")?,
            unit: row.get("unit")?,
            accept: row.get("accept")?,
            diff,
            ack_label: row.get("ack_label")?,
            items,
            slots,
            mute_key: row.get("mute_key")?,
            caller_id: row.get("caller_id")?,
        })
    }
}
