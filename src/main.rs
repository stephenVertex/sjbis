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
// No extra io imports needed

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
        cli::Commands::Rule { command } => cmd_rule(command).await,
        cli::Commands::Daemon { command } => cmd_daemon(command).await,
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
        // For now, diff requires piping via stdin — we don't read it here,
        // the caller is expected to provide it in the detail or a future field.
        None
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
                let db_path = cli::db_path();
                let api_key = std::env::var("FIREWORKS_API_KEY").ok();
                println!("Starting daemon on port {}...", port);
                daemon::run_daemon(db_path, port, api_key).await?;
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
