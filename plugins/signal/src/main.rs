use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

#[derive(Parser)]
#[command(name = "sjbis-signal")]
#[command(about = "SJBIS Signal plugin — surfaces Signal messages as notifications")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the full daemon (connects to signal-cli and surfaces questions)
    Run,
    /// Test connectivity to signal-cli and show what would be surfaced
    Test {
        /// Look back this many minutes (default: 60)
        #[arg(short, long, default_value = "60")]
        minutes: i64,
    },
    /// Send a test message via Signal (for reply testing)
    Send {
        /// Recipient phone number
        #[arg(short, long)]
        to: String,
        /// Message text
        #[arg(short, long)]
        text: String,
    },
}

#[derive(Debug, Clone)]
struct Config {
    sjbis_binary: String,
    signal_cli_binary: String,
    signal_account: String,
    poll_interval_secs: u64,
    agent_name: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sjbis_binary: "sjbis".to_string(),
            signal_cli_binary: "signal-cli".to_string(),
            signal_account: "+1234567890".to_string(),  // User should set this
            poll_interval_secs: 10,
            agent_name: "Signal".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SignalMessage {
    #[serde(rename = "envelope")]
    envelope: SignalEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
struct SignalEnvelope {
    #[serde(rename = "sourceNumber")]
    source_number: Option<String>,
    #[serde(rename = "sourceName")]
    source_name: Option<String>,
    #[serde(rename = "timestamp")]
    timestamp: i64,
    #[serde(rename = "dataMessage")]
    data_message: Option<DataMessage>,
    #[serde(rename = "syncMessage")]
    sync_message: Option<SyncMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct DataMessage {
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SyncMessage {
    #[serde(rename = "sentMessage")]
    sent_message: Option<SentMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct SentMessage {
    message: Option<String>,
    #[serde(rename = "destinationNumber")]
    destination_number: Option<String>,
}

#[derive(Debug, Clone)]
struct Message {
    sender: String,     // phone number or name
    text: String,
    timestamp: DateTime<Utc>,
    is_from_me: bool,
}

/// Fetch messages from signal-cli via JSON-RPC or receive command
async fn fetch_messages(config: &Config, since: DateTime<Utc>) -> Result<Vec<Message>> {
    // Method 1: Try signal-cli JSON-RPC if daemon is running
    // signal-cli daemon listens on a socket (unix socket or tcp)
    // We can use `signal-cli -a ACCOUNT receive --json` for polling

    let output = Command::new(&config.signal_cli_binary)
        .arg("-a")
        .arg(&config.signal_account)
        .arg("receive")
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run signal-cli receive")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User is not registered") {
            anyhow::bail!("Signal account {} is not registered. Run: signal-cli -a {} register or link", config.signal_account, config.signal_account);
        }
        anyhow::bail!("signal-cli error: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut messages = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        let envelope: SignalEnvelope = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(e) => {
                debug!("Failed to parse signal envelope: {} — {}", e, line);
                continue;
            }
        };

        // Skip sync messages (our own messages sent from other devices)
        if envelope.sync_message.is_some() {
            continue;
        }

        let text = envelope
            .data_message
            .as_ref()
            .and_then(|dm| dm.message.clone())
            .unwrap_or_default();

        if text.is_empty() {
            continue;
        }

        let sender = envelope
            .source_name
            .clone()
            .or(envelope.source_number.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let ts = Utc.timestamp_millis_opt(envelope.timestamp).unwrap_or(Utc::now());

        if ts < since {
            continue;
        }

        messages.push(Message {
            sender,
            text,
            timestamp: ts,
            is_from_me: false,
        });
    }

    Ok(messages)
}

async fn surface_question(
    sjbis_binary: &str,
    agent_name: &str,
    message: &Message,
) -> Result<Option<String>> {
    // Use the same question filter heuristic as iMessage
    if !looks_like_question(&message.text) {
        debug!("Message does not look like a question, skipping: {}", message.text);
        return Ok(None);
    }

    info!("Surfacing Signal message from {}: {}", message.sender, message.text);

    let mut cmd = Command::new(sjbis_binary);
    cmd.arg("ask")
        .arg("--question")
        .arg(&message.text)
        .arg("--yesno")
        .arg("--blocking")
        .arg("--json")
        .arg("--agent-name")
        .arg(agent_name)
        .arg("--instance")
        .arg(&message.sender)
        .arg("--detail")
        .arg(format!("Signal from {} at {}", message.sender, message.timestamp))
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
    config: &Config,
    recipient: &str,
    text: &str,
) -> Result<()> {
    info!("Sending Signal reply to {}: {}", recipient, text);

    let output = Command::new(&config.signal_cli_binary)
        .arg("-a")
        .arg(&config.signal_account)
        .arg("send")
        .arg("-m")
        .arg(text)
        .arg(recipient)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run signal-cli send")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("signal-cli send failed: {}", stderr);
    }

    info!("Signal reply sent to {}", recipient);
    Ok(())
}

/// Simple heuristic: does this look like a question?
fn looks_like_question(text: &str) -> bool {
    let lower = text.to_lowercase();

    // Explicit question marks
    if text.contains('?') {
        return true;
    }

    // Question patterns
    let question_starts = [
        "can you", "could you", "will you", "would you",
        "what do you think", "wdyt", "opinion",
        "should i", "should we", "do you want",
        "are you", "is it", "do you think",
    ];
    for pat in &question_starts {
        if lower.contains(pat) {
            return true;
        }
    }

    // Binary choice patterns
    if lower.contains(" or ") {
        // "this or that" patterns are usually questions
        return true;
    }

    false
}

async fn run_daemon() -> Result<()> {
    info!("SJBIS Signal Plugin starting...");
    info!("Make sure signal-cli is installed and linked to your account.");
    info!("Run 'signal-cli -a YOURNUMBER register' or 'signal-cli link' first.");

    let config = Config::default();
    let mut ticker = interval(Duration::from_secs(config.poll_interval_secs));
    let mut last_check = Utc::now();

    // Verify signal-cli is available
    match Command::new(&config.signal_cli_binary).arg("--version").output().await {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout);
            info!("signal-cli version: {}", version.trim());
        }
        _ => {
            warn!("signal-cli not found. Install it first:");
            warn!("  brew install signal-cli  (macOS with Homebrew)");
            warn!("  or download from https://github.com/AsamK/signal-cli/releases");
            warn!("  Then register: signal-cli -a YOURNUMBER register");
        }
    }

    loop {
        ticker.tick().await;

        match fetch_messages(&config, last_check).await {
            Ok(messages) => {
                last_check = Utc::now();
                for msg in messages {
                    match surface_question(&config.sjbis_binary, &config.agent_name, &msg).await {
                        Ok(Some(answer)) => {
                            info!("Got answer: {}", answer);
                            // Send reply back to the sender
                            // We need the phone number, not just the name
                            // For now, log that we would send it
                            info!("Would send reply to {}: {}", msg.sender, answer);
                            // send_reply(&config, &msg.sender, &answer).await?;
                        }
                        Ok(None) => {
                            debug!("No answer or not a question");
                        }
                        Err(e) => {
                            error!("Failed to surface question: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Signal fetch failed: {}", e);
                // If the error is about not being registered, don't retry rapidly
                if e.to_string().contains("not registered") {
                    sleep(Duration::from_secs(300)).await;
                }
            }
        }
    }
}

async fn run_test(minutes: i64) -> Result<()> {
    info!("Running Signal test (last {} minutes)...", minutes);

    let config = Config::default();
    let since = Utc::now() - chrono::Duration::minutes(minutes);

    match fetch_messages(&config, since).await {
        Ok(messages) => {
            println!("\nFetched {} Signal messages\n", messages.len());

            let mut questions_found = 0;
            for msg in &messages {
                let is_question = looks_like_question(&msg.text);
                if is_question {
                    questions_found += 1;
                    println!("[QUESTION] {} | {} | {}", msg.timestamp, msg.sender, msg.text);
                } else {
                    println!("[skip]     {} | {} | {}", msg.timestamp, msg.sender, msg.text);
                }
            }

            println!("\n{} of {} messages look like questions", questions_found, messages.len());

            if messages.is_empty() {
                println!("\nNo messages found. Make sure:");
                println!("  1. signal-cli is installed: signal-cli --version");
                println!("  2. Your account is registered: signal-cli -a YOURNUMBER register");
                println!("  3. There are actual Signal messages in the last {} minutes", minutes);
            }
        }
        Err(e) => {
            println!("Error fetching messages: {}", e);
            println!("\nSetup instructions:");
            println!("  1. Install signal-cli:");
            println!("     brew install signal-cli  (macOS)");
            println!("     or download from https://github.com/AsamK/signal-cli/releases");
            println!("  2. Register or link your account:");
            println!("     signal-cli -a +1234567890 register");
            println!("     or link as secondary device:");
            println!("     signal-cli link --name sjbis-signal");
            println!("  3. Verify with: signal-cli -a +1234567890 receive");
        }
    }

    Ok(())
}

async fn run_send(to: String, text: String) -> Result<()> {
    let config = Config::default();
    send_reply(&config, &to, &text).await?;
    println!("Sent Signal message to {}", to);
    Ok(())
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
        Commands::Send { to, text } => run_send(to, text).await,
    }
}
