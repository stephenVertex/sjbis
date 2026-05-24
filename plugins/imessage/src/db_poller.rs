use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, Row};
use tracing::{debug, warn};

use crate::Message;

fn row_to_message(row: &Row) -> rusqlite::Result<Message> {
    let rowid: i64 = row.get(0)?;
    let handle_id: i64 = row.get(1)?;
    let handle_text: Option<String> = row.get(2)?;
    let text: String = row.get(3)?;
    let date_raw: i64 = row.get(4)?;
    let is_from_me: bool = row.get(5)?;

    // macOS Messages app stores nanoseconds since 2001-01-01 (Apple epoch)
    // Apple epoch = 2001-01-01 00:00:00 UTC = 978307200 seconds since Unix epoch
    let apple_epoch_nanos = 978307200i64 * 1_000_000_000i64;
    let date = Utc.timestamp_nanos(apple_epoch_nanos + date_raw);

    // Resolve handle ID to phone/email
    let handle = handle_text.unwrap_or_else(|| format!("handle:{}", handle_id));

    Ok(Message {
        rowid,
        handle,
        text,
        date,
        is_from_me,
    })
}

pub async fn fetch_new_messages(since: DateTime<Utc>) -> Result<Vec<Message>> {
    let db_path = dirs::home_dir()
        .context("No home directory")?
        .join("Library/Messages/chat.db");

    if !db_path.exists() {
        return Err(anyhow::anyhow!(
            "Messages database not found at {:?}. Ensure Full Disk Access is granted.",
            db_path
        ));
    }

    // Use tokio::task::spawn_blocking for sync SQLite
    let since_clone = since;
    let messages = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&db_path)?;

        // Apple epoch = 2001-01-01 00:00:00 UTC in nanoseconds since Unix epoch
        let apple_epoch_nanos = 978307200i64 * 1_000_000_000i64;
        let since_apple = since_clone.timestamp_nanos() - apple_epoch_nanos;

        let mut stmt = conn.prepare(
            "SELECT message.ROWID, message.handle_id, COALESCE(handle.id, 'handle:' || message.handle_id), message.text, message.date, message.is_from_me 
             FROM message 
             LEFT JOIN handle ON message.handle_id = handle.ROWID
             WHERE message.date > ? 
             AND message.text IS NOT NULL 
             AND message.is_from_me = 0
             ORDER BY message.date ASC"
        )?;

        let messages: Vec<Message> = stmt
            .query_map([since_apple], row_to_message)?
            .filter_map(|r| {
                match r {
                    Ok(m) => Some(m),
                    Err(e) => {
                        warn!("Bad message row: {}", e);
                        None
                    }
                }
            })
            .collect();

        Ok::<_, anyhow::Error>(messages)
    })
    .await
    .context("DB query task panicked")?;

    debug!("Fetched {} new messages", messages.as_ref().map(|m| m.len()).unwrap_or(0));
    messages
}
