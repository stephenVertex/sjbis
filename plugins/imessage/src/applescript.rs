use anyhow::{Context, Result};
use chrono::Utc;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::Message;

/// Fetch recent messages via AppleScript.
/// This does NOT require Full Disk Access for our binary — it delegates
/// to the Messages app which already has the necessary entitlements.
pub async fn fetch_via_applescript(minutes: i64) -> Result<Vec<Message>> {
    let _since = Utc::now() - chrono::Duration::minutes(minutes);

    let script = format!(r#"
        tell application "Messages"
            set results to {{}}
            set cutoff to (current date) - ({} * minutes)
            
            repeat with aChat in chats
                try
                    repeat with aMsg in (messages of aChat whose date > cutoff)
                        try
                            set msgText to text of aMsg
                            set msgDate to date of aMsg
                            set msgSender to handle of aMsg
                            
                            try
                                set senderName to full name of msgSender
                            on error
                                try
                                    set senderName to name of msgSender
                                on error
                                    set senderName to id of msgSender
                                end try
                            end try
                            
                            set end of results to {{|text|:msgText, |date|:msgDate, |sender|:senderName, |fromMe|:senderName is "me"}}
                        on error
                            -- skip bad message
                        end try
                    end repeat
                on error
                    -- skip bad chat
                end try
            end repeat
            
            return results as string
        end tell
    "#, minutes);

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("Failed to run AppleScript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("AppleScript failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    debug!("AppleScript output: {}", stdout);

    // Parse the AppleScript list-of-records output
    // AppleScript returns: {{text:"hello", date:date "...", sender:"...", fromMe:false}, {...}}
    let messages = parse_applescript_output(&stdout)?;
    Ok(messages)
}

/// Alternative: simpler script that just gets the 20 most recent messages
pub async fn fetch_recent_simple(count: i32) -> Result<Vec<Message>> {
    let script = format!(r#"
        tell application "Messages"
            set results to {{}}
            set allMessages to {{}}
            
            repeat with aChat in chats
                try
                    set chatMsgs to messages of aChat
                    repeat with aMsg in chatMsgs
                        try
                            set end of allMessages to aMsg
                        end try
                    end repeat
                end try
            end repeat
            
            -- Sort by date (newest first)
            set sortedMsgs to my sortMessagesByDate(allMessages)
            
            -- Take last {}
            set recentMsgs to items 1 thru (minimum of {{count of sortedMsgs, {}}}) of sortedMsgs
            
            repeat with aMsg in recentMsgs
                try
                    set msgText to text of aMsg
                    set msgDate to date of aMsg
                    try
                        set msgSender to handle of aMsg
                        try
                            set senderName to full name of msgSender
                        on error
                            try
                                set senderName to name of msgSender
                            on error
                                set senderName to id of msgSender
                            end try
                        end try
                    on error
                        set senderName to "unknown"
                    end try
                    
                    set end of results to {{|text|:msgText, |date|:msgDate, |sender|:senderName}}
                end try
            end repeat
            
            return results as string
        end tell
        
        on sortMessagesByDate(msgList)
            set sortedList to msgList
            -- Simple bubble sort by date
            repeat with i from 1 to count of sortedList
                repeat with j from 1 to (count of sortedList) - i
                    set msgA to item j of sortedList
                    set msgB to item (j + 1) of sortedList
                    if date of msgA < date of msgB then
                        set item j of sortedList to msgB
                        set item (j + 1) of sortedList to msgA
                    end if
                end repeat
            end repeat
            return sortedList
        end sortMessagesByDate
    "#, count, count);

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("Failed to run AppleScript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("AppleScript failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let messages = parse_applescript_output(&stdout)?;
    Ok(messages)
}

/// Send an iMessage reply via AppleScript.
/// The Messages app must be running and Automation permissions must be granted.
pub async fn send_message(handle: &str, text: &str) -> Result<()> {
    let script = format!(
        r#"
        tell application "Messages"
            set targetService to 1st service whose service type = iMessage
            set targetBuddy to buddy "{}" of targetService
            send "{}" to targetBuddy
        end tell
    "#,
        handle.replace('"', "\\\""),
        text.replace('"', "\\\"")
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("Failed to run AppleScript send")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("AppleScript send failed: {}", stderr));
    }

    info!("Sent iMessage reply to {}", handle);
    Ok(())
}

fn parse_applescript_output(output: &str) -> Result<Vec<Message>> {
    // AppleScript record format is tricky to parse generically.
    // For now, return empty and warn that parsing needs work.
    // In production, we'd use JSON output from a JXA (JavaScript for Automation) script instead.
    warn!("AppleScript parsing not yet fully implemented. Raw output preview: {}", &output[..output.len().min(200)]);
    Ok(vec![])
}
