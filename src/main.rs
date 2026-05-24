mod cli;
mod daemon;
mod db;
mod handlers;
mod models;
mod router;
mod rules;
mod sse;

use anyhow::{Context, Result};
use clap::Parser;
use models::*;

/// Generate a synthetic diff preview from detail + question text when no
/// explicit diff is provided. Produces a few context / add / del lines.
fn generate_synthetic_diff(detail: &Option<String>, question: &str) -> Vec<DiffLine> {
    let mut lines = vec![
        DiffLine { kind: "meta".to_string(), text: format!("diff --git a/{}.rs b/{}.rs", question.split_whitespace().next().unwrap_or("file").to_lowercase(), question.split_whitespace().next().unwrap_or("file").to_lowercase()) },
        DiffLine { kind: "meta".to_string(), text: "index 0000000..1111111 100644".to_string() },
        DiffLine { kind: "meta".to_string(), text: "--- a/old.rs".to_string() },
        DiffLine { kind: "meta".to_string(), text: "+++ b/new.rs".to_string() },
    ];
    if let Some(d) = detail {
        for (i, sentence) in d.split_terminator('.').enumerate().take(4) {
            let trimmed = sentence.trim();
            if trimmed.is_empty() { continue; }
            let kind = if i % 2 == 0 { "del" } else { "add" };
            lines.push(DiffLine {
                kind: kind.to_string(),
                text: format!("- {}", trimmed),
            });
            lines.push(DiffLine {
                kind: kind.to_string(),
                text: format!("+ {}", trimmed),
            });
        }
    }
    if lines.len() == 4 {
        // detail was empty — add a placeholder
        lines.push(DiffLine { kind: "ctx".to_string(), text: "@@ -1,10 +1,10 @@".to_string() });
        lines.push(DiffLine { kind: "del".to_string(), text: "- old implementation".to_string() });
        lines.push(DiffLine { kind: "add".to_string(), text: "+ new implementation".to_string() });
    }
    lines
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info,sjbis=debug"))
        .init();

    let args = cli::Cli::parse();

    match args.command {
        cli::Commands::Ask(ask_args) => cmd_ask(ask_args).await,
        cli::Commands::List { json } => cmd_list(json).await,
        cli::Commands::Cancel { id } => cmd_cancel(id).await,
        cli::Commands::Wait { id } => cmd_wait(id).await,
        cli::Commands::Status { id } => cmd_status(id).await,
        cli::Commands::Rule { command } => cmd_rule(command).await,
        cli::Commands::Daemon { command } => cmd_daemon(command).await,
        cli::Commands::Prime => cmd_prime().await,
        cli::Commands::Register { agent_name, glyph, color } => cmd_register(agent_name, glyph, color).await,
    }
}

async fn cmd_ask(args: cli::AskArgs) -> Result<()> {
    let url = cli::daemon_url(args.daemon.clone());
    let client = reqwest::Client::new();

    // Build choices if multichoice
    let choices = if let Some(ref raw) = args.choices {
        if raw.starts_with('[') {
            serde_json::from_str(raw).ok()
        } else {
            // CSV: "a,b,c"
            let parts: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
            Some(parts.into_iter().map(|label| Choice { value: label.clone(), label, hint: None }).collect())
        }
    } else {
        None
    };

    // Build suggestions
    let suggestions = args.suggestions.as_ref().map(|s| {
        s.lines().map(|l| l.to_string()).collect::<Vec<_>>()
    });

    // Build pick items from file path
    let items = if let Some(ref path) = args.pick {
        let content = tokio::fs::read_to_string(path).await.ok();
        content.and_then(|c| serde_json::from_str::<Vec<PickItem>>(&c).ok())
    } else {
        None
    };

    // Build slots from file path
    let slots = if let Some(ref path) = args.schedule {
        let content = tokio::fs::read_to_string(path).await.ok();
        content.and_then(|c| serde_json::from_str::<Vec<Slot>>(&c).ok())
    } else {
        None
    };

    // Build diff lines from stdin or path
    let diff = if args.diff {
        // Auto-generate a synthetic diff preview from detail text when no
        // explicit diff is provided (e.g. via stdin or a future --diff-file).
        Some(generate_synthetic_diff(&args.detail, &args.question))
    } else {
        None
    };

    let req = AskRequest {
        question: args.question.clone(),
        agent_name: args.agent_name.clone(),
        instance: args.instance.clone(),
        detail: args.detail.clone(),
        urgency: args.urgency,
        blocking: args.blocking,
        deadline: args.deadline.clone(),
        reply_to: args.reply_to.clone(),
        id: args.id.clone(),
        question_type: args.question_type(),
        choices,
        yes_label: args.yes_label.clone(),
        no_label: args.no_label.clone(),
        placeholder: args.placeholder.clone(),
        suggestions,
        min: args.min,
        max: args.max,
        step: Some(args.step),
        default_value: args.default,
        unit: args.unit.clone(),
        accept: args.accept.clone(),
        diff,
        ack_label: None,
        items,
        slots,
        mute_key: args.mute_key.clone(),
        privacy: args.privacy.clone(),
    };

    let resp = client
        .post(format!("{}/ask", url))
        .json(&req)
        .send()
        .await
        .context("failed to connect to daemon")?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("daemon error: {}", text);
    }

    let notif: Notification = resp.json().await.context("failed to parse daemon response")?;

    if args.blocking {
        // Blocking mode: poll SSE or poll /list until answered or timed out
        println!("Waiting for answer... (id: {})", notif.id);
        let answer = wait_for_answer(&client, &url, &notif.id).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&answer)?);
        } else {
            println!("{}", answer.answer.unwrap_or_default());
        }
    } else {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&notif)?);
        } else {
            println!("Posted: {} (id: {})", notif.question, notif.id);
        }
    }
    Ok(())
}

async fn wait_for_answer(client: &reqwest::Client, url: &str, id: &str) -> Result<AnswerEnvelope> {
    // Use the server's blocking wait endpoint
    let resp = client
        .get(format!("{}/wait/{}", url, id))
        .timeout(std::time::Duration::from_secs(360))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("wait failed: {}", resp.text().await.unwrap_or_default());
    }
    let envelope: AnswerEnvelope = resp.json().await?;
    Ok(envelope)
}

async fn cmd_list(json: bool) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let resp = client.get(format!("{}/list", url)).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    let notifs: Vec<Notification> = resp.json().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&notifs)?);
    } else {
        if notifs.is_empty() {
            println!("No open notifications.");
        } else {
            for n in notifs {
                println!("[{}] {} | {} | {}", n.id, n.urgency, n.agent_name, n.question);
            }
        }
    }
    Ok(())
}

async fn cmd_cancel(id: String) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let resp = client.delete(format!("{}/cancel/{}", url, id)).send().await?;
    if resp.status().is_success() {
        println!("Cancelled {}", id);
    } else {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

async fn cmd_wait(id: String) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let answer = wait_for_answer(&client, &url, &id).await?;
    println!("{}", serde_json::to_string_pretty(&answer)?);
    Ok(())
}

async fn cmd_status(id: String) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let resp = client.get(format!("{}/notification/{}", url, id)).send().await?;
    if resp.status().is_success() {
        let notif: Notification = resp.json().await?;
        let status_str = match notif.status {
            NotificationStatus::Open => "open",
            NotificationStatus::Answered => "answered",
            NotificationStatus::Cancelled => "cancelled",
            NotificationStatus::Muted => "muted",
            NotificationStatus::TimedOut => "timed_out",
        };
        println!("id:        {}", notif.id);
        println!("status:    {}", status_str);
        println!("agent:     {}", notif.agent_name);
        println!("question:  {}", notif.question);
        if let Some(ref answer) = notif.answer {
            println!("answer:    {}", answer);
        }
        if let Some(ref note) = notif.note {
            println!("note:      {}", note);
        }
        if let Some(ref answered_at) = notif.answered_at {
            println!("answered:  {}", answered_at);
        }
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("notification {} not found", id);
    } else {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

async fn cmd_rule(command: cli::RuleCommands) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    match command {
        cli::RuleCommands::Add { text, scope, urgency_min, mute } => {
            let body = serde_json::json!({
                "text": text,
                "scope": scope,
                "urgency_min": urgency_min,
                "mute": mute,
            });
            let resp = client.post(format!("{}/rules", url)).json(&body).send().await?;
            if resp.status().is_success() {
                let rule: Rule = resp.json().await?;
                println!("Added rule {}: {}", rule.id, rule.text);
            } else {
                anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
            }
        }
        cli::RuleCommands::List => {
            let resp = client.get(format!("{}/state", url)).send().await?;
            let state: DashboardState = resp.json().await?;
            for r in state.rules {
                let status = if r.active { "active" } else { "inactive" };
                println!("[{}] {} — {}", r.id, status, r.text);
            }
        }
        cli::RuleCommands::Rm { id: rule_id } => {
            let resp = client.delete(format!("{}/rules/{}", url, rule_id)).send().await?;
            if resp.status().is_success() {
                println!("Removed rule {}", rule_id);
            } else {
                anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
            }
        }
    }
    Ok(())
}

async fn cmd_daemon(command: cli::DaemonCommands) -> Result<()> {
    match command {
        cli::DaemonCommands::Start { port, background } => {
            if background {
                // Spawn detached process
                let exe = std::env::current_exe()?;
                let mut cmd = std::process::Command::new(exe);
                cmd.arg("daemon").arg("start").arg("--port").arg(port.to_string());
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    cmd.process_group(0);
                }
                let child = cmd.spawn()?;
                let pid = child.id();
                std::fs::write(cli::pidfile_path(), pid.to_string())?;
                println!("Daemon started on port {} (pid {})", port, pid);
            } else {
                // Run inline
                let api_key = std::env::var("FIREWORKS_API_KEY").ok();
                println!("Starting daemon on port {}...", port);
                daemon::run_daemon(port, api_key).await?;
            }
        }
        cli::DaemonCommands::Stop => {
            let pidfile = cli::pidfile_path();
            if !pidfile.exists() {
                println!("Daemon not running (no pidfile)");
                return Ok(());
            }
            let pid_str = std::fs::read_to_string(&pidfile)?;
            let pid: u32 = pid_str.trim().parse()?;
            #[cfg(unix)]
            {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            #[cfg(not(unix))]
            {
                println!("Please stop process {} manually", pid);
            }
            let _ = std::fs::remove_file(&pidfile);
            println!("Stopped daemon (pid {})", pid);
        }
        cli::DaemonCommands::Status => {
            let pidfile = cli::pidfile_path();
            if !pidfile.exists() {
                println!("Daemon: not running");
                return Ok(());
            }
            let pid_str = std::fs::read_to_string(&pidfile)?;
            let pid: u32 = pid_str.trim().parse()?;
            // Check if process is alive
            let url = cli::daemon_url(None);
            let client = reqwest::Client::new();
            match client.get(format!("{}/health", url)).timeout(std::time::Duration::from_secs(2)).send().await {
                Ok(resp) if resp.status().is_success() => println!("Daemon: running on {} (pid {})", url, pid),
                _ => println!("Daemon: pid {} exists but not responding", pid),
            }
        }
    }
    Ok(())
}

async fn cmd_prime() -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let daemon_ok = match client.get(format!("{}/health", url)).timeout(std::time::Duration::from_secs(2)).send().await {
        Ok(resp) if resp.status().is_success() => true,
        _ => false,
    };

    let status_banner = if daemon_ok {
        format!("  Daemon: running on {}", url)
    } else {
        "  Daemon: NOT RUNNING — start it with: sjbis daemon start --port 7878".to_string()
    };

    let help_body = r#"  The surfacer is a dashboard where humans see and answer questions
  from agents (scripts, tools, AI systems). Agents post questions via
  the CLI. The daemon must be running first.

STARTING THE DAEMON
  sjbis daemon start --port 7878
  sjbis daemon start --port 7878 --background

POSTING A QUESTION (fire-and-forget)
  sjbis ask --question "Deploy to prod?" --yesno --agent-name deploybot
  sjbis ask --question "Lunch cuisine?" --choices "thai,indian,salad" --agent-name lunchbot

POSTING A QUESTION (synchronous / blocking)
  Add --blocking to wait for the human answer. The command does not
  return until the user responds on the dashboard or the deadline hits.

  sjbis ask --question "Approve PR #412?" --yesno --blocking --agent-name codebot

  The answer is printed to stdout when it arrives. Use --json for
  structured output (includes latency_ms, answer_label, etc.).

READING THE ANSWER
  Always check the `note` field in the response. Humans can attach a
  free-text note to any answer type — it may contain a follow-up
  question, rejection reason, or extra context you need to act on.

  Example response:
    {"answer":"No","note":"I need the security scan report first"}

QUESTION TYPES
  --yesno           Yes / No
  --text            Free text reply
  --number          Numeric slider (use --min, --max, --step, --unit)
  --choices "a,b,c" Multi-choice (CSV) or JSON array
  --ack             Acknowledge-only (no data collected)
  --file            File upload request (use --accept ".pdf,.csv")
  --diff            Approve/reject a code diff
  --pick <file>     Pick from a list of items (JSON file)
  --schedule <file> Pick a time slot (JSON file)

COMMON FLAGS
  --agent-name      Required. Identifies the calling agent.
  --instance        Context, e.g. "Gmail inbox", "Session s7b3d11"
  --detail          Full paragraph of context for the human.
  --urgency 0..5    0 = FYI, 5 = drop everything.
  --deadline 6m     Duration (90s, 6m, 2h) or ISO timestamp.
  --reply-to        stdout | webhook:URL | file:PATH | exit-code
  --id <key>        Idempotency key. Same key within 24h = dedupe.
  --json            Output raw JSON.

EXAMPLES
  # Security alert
  sjbis ask --question "New device signed into GitHub." --ack \
    --agent-name Sentinel --instance "GitHub" --urgency 4

  # Schedule picker
  echo '[{"day":"Mon","time":"10:00 AM"}]' > slots.json
  sjbis ask --question "Book the 1:1?" --schedule slots.json \
    --agent-name Chronos --instance "Calendar" --blocking

  # Numeric
  sjbis ask --question "How many cartons?" --number \
    --min 0 --max 8 --step 1 --default 2 --unit cartons \
    --agent-name Shopper

DASHBOARD
  Open http://localhost:7878 in a browser. Click cards to answer.
  Keyboard: J/K navigate, Enter open, 1-9 answer.

LIST / STATUS / CANCEL
  sjbis list                    Show open notifications
  sjbis status sjbis-AbCdEfGh   Check state of any notification (open/answered/cancelled/timed_out)
  sjbis cancel sjbis-AbCdEfGh   Cancel an open notification
"#;
    println!("SJBIS — How to ask questions\n\n{}\n\n{}", status_banner, help_body);
    Ok(())
}

async fn cmd_register(agent_name: String, glyph: Option<String>, color: Option<String>) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let agent = Agent {
        name: agent_name.clone(),
        glyph: glyph.unwrap_or_else(|| "◐".to_string()),
        color: color.unwrap_or_else(|| agent_color(&agent_name)),
        kind: "custom".to_string(),
    };
    let resp = client.post(format!("{}/agents", url)).json(&agent).send().await?;
    if resp.status().is_success() {
        println!("Registered agent '{}' with glyph '{}' and color {}", agent.name, agent.glyph, agent.color);
    } else {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}
