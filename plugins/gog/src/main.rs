use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

mod ai_classifier;
mod question_filter;

#[derive(Parser)]
#[command(name = "sjbis-gog")]
#[command(about = "SJBIS Google plugin — surfaces Gmail & Chat as notifications")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full daemon (polls Gmail and Chat, surfaces questions)
    Run {
        /// Gog profile(s) to use (default: auto-detect all)
        #[arg(long)]
        profile: Vec<String>,
    },
    /// Test Gmail connectivity and show what would be surfaced
    TestGmail {
        /// Gog profile to use (default: first available)
        #[arg(long)]
        profile: Option<String>,
    },
    /// Test Chat connectivity and show what would be surfaced
    TestChat {
        /// Gog profile to use
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    sjbis_binary: String,
    gog_binary: String,
    /// Poll interval in seconds
    poll_interval_secs: u64,
    /// Dedup window in seconds
    dedup_window_secs: u64,
    /// Default agent name for notifications
    agent_name: String,
    /// Gmail profiles to monitor (empty = all available)
    profiles: Vec<String>,
    /// Enable Gmail polling
    #[serde(default = "default_true")]
    enable_gmail: bool,
    /// Enable Chat polling
    #[serde(default = "default_true")]
    enable_chat: bool,
}

fn default_true() -> bool { true }

impl Default for Config {
    fn default() -> Self {
        Self {
            sjbis_binary: "sjbis".to_string(),
            gog_binary: "gog".to_string(),
            poll_interval_secs: 60,
            dedup_window_secs: 300,
            agent_name: "Gog".to_string(),
            profiles: Vec::new(),
            enable_gmail: true,
            enable_chat: true,
        }
    }
}

/// Load daemon URL from ~/.config/sjbis/daemon.toml
/// A dedup cache that prevents surfacing the same message twice within a window.
#[derive(Clone)]
struct DedupCache {
    seen: std::collections::HashSet<String>,
    window_secs: u64,
}

impl DedupCache {
    fn new(window_secs: u64) -> Self {
        Self {
            seen: std::collections::HashSet::new(),
            window_secs,
        }
    }

    fn is_duplicate(&self, id: &str) -> bool {
        self.seen.contains(id)
    }

    fn insert(&mut self, id: &str) {
        self.seen.insert(id.to_string());
    }
}

/// Parsed Gmail thread
#[derive(Debug, Clone)]
struct GmailThread {
    id: String,
    body: String,
    subject: String,
    from: String,
    date: DateTime<Utc>,
    profile: String,
}

/// Parsed Chat message
#[derive(Debug, Clone)]
struct ChatMessage {
    id: String,
    text: String,
    sender: String,
    space: String,
    date: DateTime<Utc>,
    profile: String,
}

/// Run gog command and return parsed JSON
async fn gog_json(config: &Config, profile: &str, args: &[&str]) -> Result<serde_json::Value> {
    let mut cmd = Command::new(&config.gog_binary);
    if !profile.is_empty() {
        cmd.arg("--client").arg(profile);
    }
    cmd.arg("-j").arg("--results-only");
    for arg in args {
        cmd.arg(arg);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd.output().await
        .with_context(|| format!("Failed to run gog {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gog error: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .with_context(|| format!("Failed to parse gog JSON: {}", stdout))?;

    Ok(parsed)
}

/// Fetch recent Gmail threads that are unread
async fn fetch_gmail_threads(config: &Config, profile: &str) -> Result<Vec<GmailThread>> {
    // Search for unread threads from last 24 hours
    let result = gog_json(config, profile, &[
        "gmail", "search",
        "is:unread newer_than:1d",
        "--max", "10",
    ]).await?;

    let mut threads = Vec::new();

    // gog returns a JSON array of threads directly
    let items = if result.is_array() {
        result.as_array().cloned().unwrap_or_default()
    } else {
        result.get("threads")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    for item in items {
        let id = item.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() { continue; }

        let subject = item.get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let from = item.get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Fetch the actual message body for question detection
        let body = match fetch_gmail_message_body(config, profile, &id).await {
            Ok(b) => b,
            Err(e) => {
                debug!("Failed to fetch body for {}: {}", id, e);
                String::new()
            }
        };

        threads.push(GmailThread {
            id,
            body,
            subject,
            from,
            date: Utc::now(), // gog doesn't expose precise date easily
            profile: profile.to_string(),
        });
    }

    Ok(threads)
}

/// Fetch the body of a Gmail message
async fn fetch_gmail_message_body(config: &Config, profile: &str, message_id: &str) -> Result<String> {
    let result = gog_json(config, profile, &[
        "gmail", "get", message_id,
    ]).await?;

    // gog returns a "body" field at the top level
    let body = result.get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(body)
}

/// Fetch recent Chat messages from all spaces
async fn fetch_chat_messages(config: &Config, profile: &str) -> Result<Vec<ChatMessage>> {
    let spaces_result = gog_json(config, profile, &[
        "chat", "spaces", "list",
    ]).await?;

    let mut messages = Vec::new();

    let spaces = spaces_result.get("spaces")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for space in spaces {
        let space_name = space.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if space_name.is_empty() { continue; }

        let space_display = space.get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(&space_name)
            .to_string();

        // Fetch messages for this space
        let msg_result = gog_json(config, profile, &[
            "chat", "messages", "list", &space_name,
        ]).await?;

        if let Some(items) = msg_result.get("messages").and_then(|v| v.as_array()) {
            for item in items {
                let id = item.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() { continue; }

                let text = item.get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let sender = item.get("sender")
                    .and_then(|v| v.get("displayName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                messages.push(ChatMessage {
                    id,
                    text,
                    sender,
                    space: space_display.clone(),
                    date: Utc::now(),
                    profile: profile.to_string(),
                });
            }
        }
    }

    Ok(messages)
}

/// Surface a question via `sjbis ask` CLI
async fn surface_question(
    sjbis_binary: &str,
    agent_name: &str,
    profile: &str,
    source: &str,
    question: &str,
    detail: &str,
) -> Result<Option<String>> {
    let mut cmd = Command::new(sjbis_binary);
    cmd.arg("ask")
        .arg("--question")
        .arg(question)
        .arg("--yesno")
        .arg("--blocking")
        .arg("--json")
        .arg("--agent-name")
        .arg(agent_name)
        .arg("--instance")
        .arg(format!("{} · {}", profile, source))
        .arg("--detail")
        .arg(detail)
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

/// Send a Gmail reply via gog
async fn send_gmail_reply(config: &Config, profile: &str, thread_id: &str, subject: &str, reply: &str) -> Result<()> {
    info!("Sending Gmail reply to thread {} via gog", thread_id);

    let reply_subject = if subject.starts_with("Re: ") {
        subject.to_string()
    } else {
        format!("Re: {}", subject)
    };

    let mut cmd = Command::new(&config.gog_binary);
    if !profile.is_empty() {
        cmd.arg("--client").arg(profile);
    }
    cmd.arg("gmail")
        .arg("send")
        .arg("--thread-id")
        .arg(thread_id)
        .arg("--reply-all")
        .arg("--subject")
        .arg(&reply_subject)
        .arg("--body")
        .arg(reply)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.context("Failed to run gog gmail send")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gog gmail send failed: {}", stderr);
    }

    info!("Gmail reply sent to thread {}", thread_id);
    Ok(())
}

/// Mark a Gmail thread as read
async fn mark_gmail_read(config: &Config, profile: &str, message_id: &str) -> Result<()> {
    info!("Marking Gmail message {} as read", message_id);

    let mut cmd = Command::new(&config.gog_binary);
    if !profile.is_empty() {
        cmd.arg("--client").arg(profile);
    }
    cmd.arg("gmail")
        .arg("mark-read")
        .arg(message_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.context("Failed to run gog gmail mark-read")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gog gmail mark-read failed: {}", stderr);
    }

    info!("Marked Gmail message {} as read", message_id);
    Ok(())
}

/// Send a Chat reply via gog
async fn send_chat_reply(config: &Config, profile: &str, space: &str, thread: &str, reply: &str) -> Result<()> {
    info!("Sending Chat reply to space {} via gog", space);

    let mut cmd = Command::new(&config.gog_binary);
    if !profile.is_empty() {
        cmd.arg("--client").arg(profile);
    }
    cmd.arg("chat")
        .arg("messages")
        .arg("create")
        .arg(space)
        .arg("--text")
        .arg(reply);

    if !thread.is_empty() {
        cmd.arg("--thread").arg(thread);
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.context("Failed to run gog chat messages create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gog chat reply failed: {}", stderr);
    }

    info!("Chat reply sent to space {}", space);
    Ok(())
}

/// Get list of available gog profiles by reading gog's config.json
async fn get_profiles(config: &Config) -> Result<Vec<String>> {
    // Read gog config.json to find all account_clients
    let gog_config_path = dirs::config_dir()
        .map(|d| d.join("gogcli").join("config.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config/gogcli/config.json"));

    let config_content = tokio::fs::read_to_string(&gog_config_path).await
        .with_context(|| format!("Failed to read gog config at {:?}", gog_config_path))?;

    let parsed: serde_json::Value = serde_json::from_str(&config_content)
        .with_context(|| "Failed to parse gog config.json")?;

    let profiles = parsed.get("account_clients")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.values()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    Ok(profiles)
}

async fn run_daemon(profiles: Vec<String>) -> Result<()> {
    info!("SJBIS Gog Plugin starting...");

    let config = Config::default();
    let mut dedup = DedupCache::new(config.dedup_window_secs);

    // Determine profiles to monitor
    let profiles = if profiles.is_empty() {
        info!("No profiles configured, auto-detecting...");
        match get_profiles(&config).await {
            Ok(p) if !p.is_empty() => {
                info!("Found profiles: {:?}", p);
                p
            }
            _ => {
                warn!("No gog profiles found. Run 'gog auth login' first.");
                vec!["default".to_string()]
            }
        }
    } else {
        profiles
    };

    if config.enable_gmail {
        for profile in &profiles {
            let config = config.clone();
            let profile = profile.clone();
            let dedup = dedup.clone();
            tokio::spawn(gmail_poller_task(config, profile, dedup));
        }
    }

    if config.enable_chat {
        for profile in &profiles {
            let config = config.clone();
            let profile = profile.clone();
            let dedup = dedup.clone();
            tokio::spawn(chat_poller_task(config, profile, dedup));
        }
    }

    // Keep main alive
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

async fn gmail_poller_task(config: Config, profile: String, mut dedup: DedupCache) {
    let mut ticker = interval(Duration::from_secs(config.poll_interval_secs));
    info!("Gmail poller for profile {} started, interval={}s", profile, config.poll_interval_secs);

    loop {
        ticker.tick().await;
        info!("Gmail poll tick for profile {}", profile);

        match fetch_gmail_threads(&config, &profile).await {
            Ok(threads) => {
                for thread in threads {
                    let dedup_key = format!("gmail:{}:{}", profile, thread.id);
                    if dedup.is_duplicate(&dedup_key) {
                        debug!("Duplicate Gmail thread, skipping: {}", thread.id);
                        continue;
                    }
                    dedup.insert(&dedup_key);

                    // Use AI to classify whether this email is a question
                    let (is_question, explanation) = match ai_classifier::is_question_for_user(
                        &thread.body,
                        &thread.from,
                        &thread.subject,
                    ).await {
                        Ok((is_q, exp)) => {
                            debug!("AI classified email from {}: is_question={} ({})", thread.from, is_q, exp);
                            (is_q, exp)
                        }
                        Err(e) => {
                            warn!("AI classifier failed for email from {}: {}, falling back to regex", thread.from, e);
                            // Fallback to regex
                            let is_q = !thread.body.is_empty() && question_filter::looks_like_question(&thread.body)
                                || question_filter::looks_like_question(&thread.subject);
                            (is_q, "regex fallback".to_string())
                        }
                    };

                    if !is_question {
                        debug!("Gmail thread not a question ({}): {} | {}", explanation, thread.from, thread.subject);
                        continue;
                    }

                    let question_text = if !thread.body.is_empty() {
                        thread.body.clone()
                    } else {
                        thread.subject.clone()
                    };

                    info!("Surfacing Gmail question from {}: {}", thread.from, question_text);

                    let detail = format!("Email: {}\nSubject: {}\nProfile: {}", thread.from, thread.subject, profile);

                    match surface_question(&config.sjbis_binary, &config.agent_name, &profile, &thread.from, &question_text, &detail).await {
                        Ok(Some(answer)) => {
                            info!("Got answer: {}", answer);
                            if let Err(e) = send_gmail_reply(&config, &profile, &thread.id, &thread.subject, &answer).await {
                                error!("Failed to send Gmail reply: {}", e);
                            } else {
                                // Mark as read after successful reply
                                if let Err(e) = mark_gmail_read(&config, &profile, &thread.id).await {
                                    warn!("Failed to mark Gmail as read: {}", e);
                                }
                            }
                        }
                        Ok(None) => {
                            debug!("No answer or dismissed");
                            // Mark as read even if dismissed without answering
                            if let Err(e) = mark_gmail_read(&config, &profile, &thread.id).await {
                                warn!("Failed to mark Gmail as read: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to surface Gmail question: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Gmail poll failed for profile {}: {}", profile, e);
                // If it's an auth error, don't retry rapidly
                if e.to_string().contains("invalid_grant") || e.to_string().contains("auth") {
                    warn!("Gmail auth error for profile {} — run 'gog auth login'", profile);
                    sleep(Duration::from_secs(600)).await;
                }
            }
        }
    }
}

async fn chat_poller_task(config: Config, profile: String, mut dedup: DedupCache) {
    let mut ticker = interval(Duration::from_secs(config.poll_interval_secs));
    info!("Chat poller for profile {} started, interval={}s", profile, config.poll_interval_secs);

    loop {
        ticker.tick().await;
        info!("Chat poll tick for profile {}", profile);

        match fetch_chat_messages(&config, &profile).await {
            Ok(messages) => {
                for msg in messages {
                    let dedup_key = format!("chat:{}:{}", profile, msg.id);
                    if dedup.is_duplicate(&dedup_key) {
                        debug!("Duplicate Chat message, skipping: {}", msg.id);
                        continue;
                    }
                    dedup.insert(&dedup_key);

                    // Use AI to classify whether this message is a question
                    let (is_question, explanation) = match ai_classifier::is_question_for_user(
                        &msg.text,
                        &msg.sender,
                        &format!("Chat in {}", msg.space),
                    ).await {
                        Ok((is_q, exp)) => {
                            debug!("AI classified chat from {}: is_question={} ({})", msg.sender, is_q, exp);
                            (is_q, exp)
                        }
                        Err(e) => {
                            warn!("AI classifier failed for chat from {}: {}, falling back to regex", msg.sender, e);
                            let is_q = question_filter::looks_like_question(&msg.text);
                            (is_q, "regex fallback".to_string())
                        }
                    };

                    if !is_question {
                        debug!("Chat message not a question ({}): {} | {}", explanation, msg.sender, msg.text);
                        continue;
                    }

                    info!("Surfacing Chat question from {} in {}: {}", msg.sender, msg.space, msg.text);

                    let detail = format!("Chat from {} in {}\nProfile: {}", msg.sender, msg.space, profile);

                    match surface_question(&config.sjbis_binary, &config.agent_name, &profile, &msg.sender, &msg.text, &detail).await {
                        Ok(Some(answer)) => {
                            info!("Got answer: {}", answer);
                            // Extract space name from message id (spaces/xxx/messages/yyy)
                            let space = if let Some(idx) = msg.id.find("/messages/") {
                                msg.id[..idx].to_string()
                            } else {
                                msg.space.clone()
                            };
                            let thread = if let Some(idx) = msg.id.find("/messages/") {
                                msg.id[idx + 10..].to_string()
                            } else {
                                String::new()
                            };
                            if let Err(e) = send_chat_reply(&config, &profile, &space, &thread, &answer).await {
                                error!("Failed to send Chat reply: {}", e);
                            }
                        }
                        Ok(None) => {
                            debug!("No answer or dismissed");
                        }
                        Err(e) => {
                            error!("Failed to surface Chat question: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Chat poll failed for profile {}: {}", profile, e);
                if e.to_string().contains("invalid_grant") || e.to_string().contains("auth") {
                    warn!("Chat auth error for profile {} — run 'gog auth login'", profile);
                    sleep(Duration::from_secs(600)).await;
                }
            }
        }
    }
}

async fn run_test_gmail(profile: Option<String>) -> Result<()> {
    info!("Running Gmail test...");

    let config = Config::default();
    let profile = match profile {
        Some(p) => p,
        None => {
            match get_profiles(&config).await {
                Ok(profiles) if !profiles.is_empty() => profiles.into_iter().next().unwrap(),
                _ => "default".to_string(),
            }
        }
    };

    info!("Using profile: {}", profile);

    match fetch_gmail_threads(&config, &profile).await {
        Ok(threads) => {
            println!("\nFound {} unread Gmail threads\n", threads.len());
            let mut questions_found = 0;
            for thread in &threads {
                let (is_question, explanation) = match ai_classifier::is_question_for_user(
                    &thread.body,
                    &thread.from,
                    &thread.subject,
                ).await {
                    Ok((is_q, exp)) => (is_q, exp),
                    Err(e) => {
                        println!("AI error for {}: {}, using regex fallback", thread.from, e);
                        let is_q = !thread.body.is_empty() && question_filter::looks_like_question(&thread.body)
                            || question_filter::looks_like_question(&thread.subject);
                        (is_q, "regex fallback".to_string())
                    }
                };
                if is_question {
                    questions_found += 1;
                    println!("[QUESTION] {} | {} | {} | {}", thread.from, thread.subject, explanation, thread.body);
                } else {
                    println!("[skip]     {} | {} | {} | {}", thread.from, thread.subject, explanation, thread.body);
                }
            }
            println!("\n{} of {} threads look like questions", questions_found, threads.len());
        }
        Err(e) => {
            println!("Error: {}", e);
            println!("\nMake sure gog is authenticated:");
            println!("  gog auth login");
        }
    }

    Ok(())
}

async fn run_test_chat(profile: Option<String>) -> Result<()> {
    info!("Running Chat test...");

    let config = Config::default();
    let profile = match profile {
        Some(p) => p,
        None => {
            match get_profiles(&config).await {
                Ok(profiles) if !profiles.is_empty() => profiles.into_iter().next().unwrap(),
                _ => "default".to_string(),
            }
        }
    };

    info!("Using profile: {}", profile);

    match fetch_chat_messages(&config, &profile).await {
        Ok(messages) => {
            println!("\nFound {} Chat messages\n", messages.len());
            let mut questions_found = 0;
            for msg in &messages {
                let (is_question, explanation) = match ai_classifier::is_question_for_user(
                    &msg.text,
                    &msg.sender,
                    &format!("Chat in {}", msg.space),
                ).await {
                    Ok((is_q, exp)) => (is_q, exp),
                    Err(e) => {
                        println!("AI error for {}: {}, using regex fallback", msg.sender, e);
                        let is_q = question_filter::looks_like_question(&msg.text);
                        (is_q, "regex fallback".to_string())
                    }
                };
                if is_question {
                    questions_found += 1;
                    println!("[QUESTION] {} | {} | {} | {}", msg.space, msg.sender, explanation, msg.text);
                } else {
                    println!("[skip]     {} | {} | {} | {}", msg.space, msg.sender, explanation, msg.text);
                }
            }
            println!("\n{} of {} messages look like questions", questions_found, messages.len());
        }
        Err(e) => {
            println!("Error: {}", e);
            println!("\nMake sure gog is authenticated:");
            println!("  gog auth login");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let cli = Cli::parse();
    let cmd = cli.command.unwrap_or(Commands::Run { profile: vec![] });

    match cmd {
        Commands::Run { profile } => run_daemon(profile).await,
        Commands::TestGmail { profile } => run_test_gmail(profile).await,
        Commands::TestChat { profile } => run_test_chat(profile).await,
    }
}
