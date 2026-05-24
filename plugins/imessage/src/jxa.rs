use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::process::Command;
use tracing::{info, warn};

use crate::Message;

/// Fetch recent messages via JXA (JavaScript for Automation).
/// Returns proper JSON that Rust can parse easily.
pub async fn fetch_via_jxa(minutes: i64) -> Result<Vec<Message>> {
    let script = format!(
        r#"
        var Messages = Application("Messages");
        var results = [];
        var cutoff = new Date(Date.now() - {} * 60 * 1000);
        
        try {{
            Messages.chats().forEach(function(chat) {{
                try {{
                    chat.messages().forEach(function(msg) {{
                        try {{
                            var msgDate = msg.date();
                            if (msgDate > cutoff) {{
                                var sender = "unknown";
                                try {{
                                    sender = msg.handle().name();
                                }} catch(e) {{
                                    try {{
                                        sender = msg.handle().id();
                                    }} catch(e2) {{
                                        sender = "unknown";
                                    }}
                                }}
                                
                                results.push({{
                                    text: msg.text() || "",
                                    date: msgDate.toISOString(),
                                    sender: sender,
                                    is_from_me: msg.isFromMe() || false
                                }});
                            }}
                        }} catch(e) {{
                            // skip bad message
                        }}
                    }});
                }} catch(e) {{
                    // skip bad chat
                }}
            }});
        }} catch(e) {{
            // Messages app not accessible
        }}
        
        JSON.stringify(results);
    "#,
        minutes
    );

    let output = Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("Failed to run JXA script")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("JXA fetch failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw_messages: Vec<RawMessage> = serde_json::from_str(&stdout)
        .context("Failed to parse JXA JSON output")?;

    let messages: Vec<Message> = raw_messages
        .into_iter()
        .filter_map(|raw| {
            let date = match DateTime::parse_from_rfc3339(&raw.date) {
                Ok(d) => d.with_timezone(&Utc),
                Err(_) => {
                    warn!("Bad date in JXA output: {}", raw.date);
                    return None;
                }
            };

            Some(Message {
                rowid: 0, // JXA doesn't expose rowid
                handle: raw.sender,
                text: raw.text,
                date,
                is_from_me: raw.is_from_me,
            })
        })
        .collect();

    info!("JXA fetched {} messages", messages.len());
    Ok(messages)
}

/// Send an iMessage reply via JXA.
/// The Messages app must be running and Automation permissions must be granted.
pub async fn send_message_jxa(handle: &str, text: &str) -> Result<()> {
    let script = format!(
        r#"
        var Messages = Application("Messages");
        var targetService = Messages.services().find(function(s) {{
            return s.serviceType() === "iMessage";
        }});
        
        if (!targetService) {{
            throw new Error("iMessage service not found");
        }}
        
        var buddy = targetService.buddies().find(function(b) {{
            return b.id() === "{}" || b.name() === "{}";
        }});
        
        if (!buddy) {{
            buddy = targetService.buddies.byId("{}");
        }}
        
        Messages.send("{}", {{ to: buddy }});
        "success";
    "#,
        handle.replace('"', "\\\""),
        handle.replace('"', "\\\""),
        handle.replace('"', "\\\""),
        text.replace('"', "\\\"")
    );

    let output = Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("Failed to run JXA send script")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("JXA send failed: {}", stderr));
    }

    info!("Sent iMessage reply to {} via JXA", handle);
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    text: String,
    date: String,
    sender: String,
    #[serde(rename = "is_from_me")]
    is_from_me: bool,
}
