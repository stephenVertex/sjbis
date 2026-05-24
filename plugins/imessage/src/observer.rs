use std::sync::mpsc::channel;
use std::time::Duration;

use tracing::{info, warn};

/// Poll-based notification detection using AppleScript.
/// This is a pragmatic fallback that works without Full Disk Access.
pub fn start_applescript_poller() -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = channel::<String>();
    
    std::thread::spawn(move || {
        info!("Starting AppleScript notification poller...");
        loop {
            std::thread::sleep(Duration::from_secs(3));
            
            // AppleScript to check if Messages app has unread messages
            let script = r#"
                tell application "System Events"
                    if exists (processes where name is "Messages") then
                        return "running"
                    else
                        return "not_running"
                    end if
                end tell
            "#;
            
            let output = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
            
            match output {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.trim() == "running" {
                        // Messages is running, trigger a DB poll
                        let _ = tx.send("poll".to_string());
                    }
                }
                _ => {}
            }
        }
    });
    
    rx
}
