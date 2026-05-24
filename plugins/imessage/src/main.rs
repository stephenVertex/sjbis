use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

mod applescript;
mod db_poller;
mod jxa;
mod observer;
mod question_filter;

#[derive(Parser)]
#[command(name = "sjbis-imessage")]
#[command(about = "SJBIS iMessage plugin")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full daemon (polls DB and surfaces questions)
    Run,
    /// Test DB connectivity and show what would be surfaced (dry run)
    Test {
        /// Look back this many minutes (default: 60)
        #[arg(short, long, default_value = "60")]
        minutes: i64,
    },
    /// Test JXA (JavaScript for Automation) message fetching
    TestJxa {
        /// Look back this many minutes (default: 60)
        #[arg(short, long, default_value = "60")]
        minutes: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    sjbis_binary: String,
    poll_interval_secs: u64,
    dedup_window_secs: u64,
    agent_name: String,
    #[serde(default)]
    enable_db_poller: bool,
    #[serde(default)]
    enable_notification_observer: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sjbis_binary: "sjbis".to_string(),
            poll_interval_secs: 5,
            dedup_window_secs: 300,
            agent_name: "iMessage".to_string(),
            enable_db_poller: true,
            enable_notification_observer: true,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct DedupKey {
    handle: String,
    text_hash: String,
}

#[allow(dead_code)]
struct DedupCache {
    seen: HashSet<DedupKey>,
    window_secs: u64,
}

impl DedupCache {
    fn new(window_secs: u64) -> Self {
        Self {
            seen: HashSet::new(),
            window_secs,
        }
    }

    fn is_duplicate(&self, handle: &str, text: &str) -> bool {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let text_hash = format!("{:x}", hasher.finish());
        let key = DedupKey {
            handle: handle.to_string(),
            text_hash,
        };
        self.seen.contains(&key)
    }

    fn insert(&mut self, handle: &str, text: &str) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let text_hash = format!("{:x}", hasher.finish());
        let key = DedupKey {
            handle: handle.to_string(),
            text_hash,
        };
        self.seen.insert(key);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Message {
    rowid: i64,
    handle: String,
    text: String,
    date: DateTime<Utc>,
    is_from_me: bool,
}

async fn surface_question(
    sjbis_binary: &str,
    agent_name: &str,
    message: &Message,
) -> Result<Option<String>> {
    if !question_filter::looks_like_question(&message.text) {
        debug!("Message does not look like a question, skipping");
        return Ok(None);
    }

    info!("Surfacing question from {}: {}", message.handle, message.text);

    let choices = question_filter::infer_choices(&message.text);
    let question_type = if !choices.is_empty() {
        "--choices"
    } else {
        "--yesno"
    };

    let mut cmd = Command::new(sjbis_binary);
    cmd.arg("ask")
        .arg("--question")
        .arg(&message.text)
        .arg(question_type);

    if !choices.is_empty() {
        cmd.arg(choices.join(","));
    }

    cmd.arg("--blocking")
        .arg("--json")
        .arg("--agent-name")
        .arg(agent_name)
        .arg("--instance")
        .arg(&message.handle)
        .arg("--detail")
        .arg(format!("iMessage from {} at {}", message.handle, message.date))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.context("Failed to run sjbis ask")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("sjbis ask failed: {}", stderr);
        return Err(anyhow::anyhow!("sjbis ask failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: serde_json::Value = serde_json::from_str(&stdout)
        .context("Failed to parse sjbis response")?;

    let answer = response.get("answer")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(answer)
}

async fn send_reply(
    handle: &str,
    text: &str,
) -> Result<()> {
    info!("Sending reply to {} via JXA", handle);
    jxa::send_message_jxa(handle, text).await
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let cli = Cli::parse();
    let cmd = cli.command.unwrap_or(Commands::Run);

    match cmd {
        Commands::Run => run_daemon().await,
        Commands::Test { minutes } => run_test(minutes).await,
        Commands::TestJxa { minutes } => run_test_jxa(minutes).await,
    }
}

async fn run_test_jxa(minutes: i64) -> Result<()> {
    info!("Running JXA test (last {} minutes)...", minutes);

    let messages = jxa::fetch_via_jxa(minutes).await?;

    println!("\nJXA fetched {} messages in the last {} minutes\n", messages.len(), minutes);

    let mut questions_found = 0;
    for msg in &messages {
        let is_question = question_filter::looks_like_question(&msg.text);
        let choices = question_filter::infer_choices(&msg.text);

        if is_question {
            questions_found += 1;
            println!("[QUESTION] {} | {} | choices={:?}",
                msg.handle, msg.text, choices);
        } else {
            println!("[skip]     {} | {}", msg.handle, msg.text);
        }
    }

    println!("\n{} of {} messages look like questions", questions_found, messages.len());
    Ok(())
}

async fn run_daemon() -> Result<()> {
    info!("SJBIS iMessage Plugin starting...");

    let config = Config::default();
    let dedup = DedupCache::new(config.dedup_window_secs);

    if config.enable_db_poller {
        info!("DB poller enabled");
        tokio::spawn(db_poller_task(config.clone(), dedup));
    }

    if config.enable_notification_observer {
        info!("Notification observer enabled (macOS only)");
        #[cfg(target_os = "macos")]
        tokio::spawn(notification_observer_task(config.clone()));
        #[cfg(not(target_os = "macos"))]
        warn!("Notification observer only works on macOS");
    }

    // Keep main alive
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

async fn run_test(minutes: i64) -> Result<()> {
    info!("Running DB test (last {} minutes)...", minutes);

    let since = Utc::now() - chrono::Duration::minutes(minutes);
    let messages = db_poller::fetch_new_messages(since).await?;

    println!("\nFound {} messages in the last {} minutes\n", messages.len(), minutes);

    let mut questions_found = 0;
    for msg in &messages {
        let is_question = question_filter::looks_like_question(&msg.text);
        let choices = question_filter::infer_choices(&msg.text);

        if is_question {
            questions_found += 1;
            println!("[QUESTION] {} | {} | choices={:?}",
                msg.handle, msg.text, choices);
        } else {
            println!("[skip]     {} | {}", msg.handle, msg.text);
        }
    }

    println!("\n{} of {} messages look like questions", questions_found, messages.len());
    
    // If no messages found, try to diagnose
    if messages.is_empty() {
        println!("\nNote: No messages found in the last {} minutes.", minutes);
        println!("This could mean:");
        println!("  1. No messages were received in this time window");
        println!("  2. Messages were deleted from the database");
        println!("  3. The Messages app database path changed");
        println!("\nTry with a larger --minutes value, e.g. --minutes 1440 (24 hours)");
    }
    
    Ok(())
}

async fn db_poller_task(config: Config, mut dedup: DedupCache) {
    let mut ticker = interval(Duration::from_secs(config.poll_interval_secs));
    let mut last_check = Utc::now();

    loop {
        ticker.tick().await;

        match db_poller::fetch_new_messages(last_check).await {
            Ok(messages) => {
                last_check = Utc::now();
                for msg in messages {
                    if dedup.is_duplicate(&msg.handle, &msg.text) {
                        debug!("Duplicate message, skipping");
                        continue;
                    }
                    dedup.insert(&msg.handle, &msg.text);

                    if msg.is_from_me {
                        continue; // Don't surface our own messages
                    }

                    match surface_question(&config.sjbis_binary, &config.agent_name, &msg).await {
                        Ok(Some(answer)) => {
                            info!("Got answer: {}", answer);
                            if let Err(e) = send_reply(&msg.handle, &answer).await {
                                error!("Failed to send reply: {}", e);
                            }
                        }
                        Ok(None) => {
                            debug!("Message surfaced but no answer or not a question");
                        }
                        Err(e) => {
                            error!("Failed to surface question: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("DB poll failed: {}", e);
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn notification_observer_task(config: Config) {
    info!("Starting AppleScript notification poller...");
    let rx = observer::start_applescript_poller();
    let mut dedup = DedupCache::new(config.dedup_window_secs);

    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(_ping) => {
                debug!("Notification poller triggered, checking for new messages");
                // Poll recent messages via AppleScript as fallback
                            match jxa::fetch_via_jxa(5).await {
                                Ok(messages) => {
                        for msg in messages {
                            if dedup.is_duplicate(&msg.handle, &msg.text) {
                                continue;
                            }
                            dedup.insert(&msg.handle, &msg.text);

                            if msg.is_from_me {
                                continue;
                            }

                            match surface_question(&config.sjbis_binary, &config.agent_name, &msg).await {
                                Ok(Some(answer)) => {
                                    info!("Got answer: {}", answer);
                                    // Phase 2: send reply back
                                    let _ = send_reply(&msg.handle, &answer).await;
                                }
                                Ok(None) => {}
                                Err(e) => error!("Failed to surface question: {}", e),
                            }
                        }
                    }
                    Err(e) => {
                        warn!("AppleScript fetch failed: {}", e);
                    }
                }
            }
            Err(_) => {
                // Timeout, loop again
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
async fn notification_observer_task(_config: Config) {
    warn!("Notification observer only available on macOS");
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}
