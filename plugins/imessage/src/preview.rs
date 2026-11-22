use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const APPLE_EPOCH_NANOS: i64 = 978_307_200 * 1_000_000_000;
const MAX_MESSAGE_AGE_NANOS: i64 = 120 * 1_000_000_000;
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const PI_RECEIVE: &str = "/usr/bin/curl --fail --silent --show-error --max-time 5 \
    -H 'Content-Type: application/json' --data-binary @- http://127.0.0.1:7881/messages";

#[derive(Deserialize)]
struct Config {
    pi_ssh: String,
    contacts: Vec<Contact>,
}

#[derive(Deserialize)]
struct Contact {
    name: String,
    handles: Vec<String>,
    photo: Option<String>,
}

struct Message {
    rowid: i64,
    guid: String,
    handle: String,
    text: String,
    date: i64,
}

#[derive(Serialize)]
struct Preview<'a> {
    id: &'a str,
    name: &'a str,
    text: String,
    photo: Option<&'a str>,
}

fn normalize_handle(handle: &str) -> String {
    let handle = handle.trim();
    if handle.contains('@') {
        return handle.to_ascii_lowercase();
    }
    let digits: String = handle.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 11 && digits.starts_with('1') {
        digits[1..].to_string()
    } else {
        digits
    }
}

fn messages_after(db: &Connection, cursor: i64) -> Result<Vec<Message>> {
    let mut query = db.prepare_cached(
        "SELECT message.ROWID, message.guid, handle.id, message.text, message.date
         FROM message JOIN handle ON message.handle_id = handle.ROWID
         WHERE message.ROWID > ? AND message.is_from_me = 0
           AND message.text IS NOT NULL AND message.date IS NOT NULL
         ORDER BY message.ROWID LIMIT 100",
    )?;
    let messages = query.query_map([cursor], |row| {
        Ok(Message {
            rowid: row.get(0)?,
            guid: row.get(1)?,
            handle: row.get(2)?,
            text: row.get(3)?,
            date: row.get(4)?,
        })
    })?;
    Ok(messages.collect::<rusqlite::Result<_>>()?)
}

async fn publish(host: &str, preview: &Preview<'_>) -> Result<()> {
    let mut ssh = Command::new("/usr/bin/ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "--",
            host,
            PI_RECEIVE,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let data = serde_json::to_vec(preview)?;
    tokio::time::timeout(Duration::from_secs(12), async {
        ssh.stdin
            .take()
            .context("SSH stdin unavailable")?
            .write_all(&data)
            .await?;
        ensure!(ssh.wait().await?.success(), "Pi rejected the preview");
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

pub async fn run(config_path: &Path) -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(config_path)?)?;
    let mut contacts = HashMap::new();
    for contact in &config.contacts {
        for handle in &contact.handles {
            let handle = normalize_handle(handle);
            ensure!(!handle.is_empty(), "Invalid contact handle");
            if let Some(previous) = contacts.insert(handle, contact) {
                ensure!(
                    std::ptr::eq(previous, contact),
                    "A handle belongs to multiple configured contacts"
                );
            }
        }
    }
    ensure!(!contacts.is_empty(), "Configure at least one contact");

    let db_path = dirs::home_dir()
        .context("No home directory")?
        .join("Library/Messages/chat.db");
    let db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("Opening Messages database read-only")?;
    let cursor_path = config_path.with_extension("cursor");
    let pending_cursor = config_path.with_extension("cursor.new");
    let mut cursor = match std::fs::read_to_string(&cursor_path) {
        Ok(value) => value
            .trim()
            .parse::<i64>()
            .context("Invalid preview cursor")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            db.query_row("SELECT COALESCE(MAX(ROWID), 0) FROM message", [], |row| {
                row.get(0)
            })?
        }
        Err(error) => return Err(error.into()),
    };
    let save_cursor = |rowid: i64| -> Result<()> {
        std::fs::write(&pending_cursor, rowid.to_string())?;
        std::fs::rename(&pending_cursor, &cursor_path)?;
        Ok(())
    };
    save_cursor(cursor)?;
    tracing::info!(
        contacts = config.contacts.len(),
        "Watching for Pi message previews"
    );

    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let messages = match messages_after(&db, cursor) {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(%error, "Could not poll Messages");
                continue;
            }
        };
        let previous_cursor = cursor;
        for message in messages {
            let cutoff = Utc::now()
                .timestamp_nanos_opt()
                .context("Invalid system time")?
                - APPLE_EPOCH_NANOS
                - MAX_MESSAGE_AGE_NANOS;
            let text = message.text.trim();
            if message.date >= cutoff && !text.is_empty() {
                if let Some(contact) = contacts.get(&normalize_handle(&message.handle)) {
                    let preview = Preview {
                        id: &message.guid,
                        name: &contact.name,
                        text: text.chars().take(600).collect(),
                        photo: contact.photo.as_deref(),
                    };
                    if let Err(error) = publish(&config.pi_ssh, &preview).await {
                        tracing::warn!(%error, "Could not send Pi preview; will retry");
                        break;
                    }
                }
            }
            cursor = message.rowid;
        }
        if cursor != previous_cursor {
            save_cursor(cursor)?;
        }
    }
}
