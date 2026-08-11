mod cli;
mod daemon;
mod db;
mod entities;
mod handlers;
mod models;
mod push;
mod router;
mod rules;
mod sse;
mod upgrade;
mod version;

use anyhow::{Context, Result};
use clap::Parser;
use models::*;

/// Unescape common escape sequences in a string (\\n → newline, \\t → tab, etc.)
fn unescape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

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
        cli::Commands::Dismiss { id } => cmd_dismiss(id).await,
        cli::Commands::Answer { id, answer, via, note } => cmd_answer(id, answer, via, note).await,
        cli::Commands::Wait { id } => cmd_wait(id).await,
        cli::Commands::Status { id } => cmd_status(id).await,
        cli::Commands::Rule { command } => cmd_rule(command).await,
        cli::Commands::Entity { command } => cmd_entity(command),
        cli::Commands::Daemon { command } => cmd_daemon(command).await,
        cli::Commands::Prime => cmd_prime().await,
        cli::Commands::Register { agent_name, glyph, color } => cmd_register(agent_name, glyph, color).await,
        cli::Commands::Upgrade { check, force, tag } => cmd_upgrade(check, force, tag).await,
    }
}

fn cmd_entity(command: cli::EntityCommands) -> Result<()> {
    let config_path = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".config/sjbis/entities.toml");
    let _ = std::fs::create_dir_all(config_path.parent().unwrap());

    let mut toml_value = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    // Ensure top-level is a table
    if !toml_value.is_table() {
        toml_value = toml::Value::Table(toml::map::Map::new());
    }

    // Ensure "groups" key exists
    let table = toml_value.as_table_mut().unwrap();
    if !table.contains_key("groups") {
        table.insert("groups".to_string(), toml::Value::Table(toml::map::Map::new()));
    }

    let groups = table.get_mut("groups").unwrap().as_table_mut().unwrap();

    match command {
        cli::EntityCommands::Add { name, members } => {
            let arr: Vec<toml::Value> = members.iter().map(|m| toml::Value::String(m.clone())).collect();
            groups.insert(name.clone(), toml::Value::Array(arr));
            let content = toml::to_string_pretty(&toml_value)?;
            std::fs::write(&config_path, content)?;
            println!("Created group '{}' with {} member(s)", name, members.len());
            println!("Config saved to {}", config_path.display());
        }
        cli::EntityCommands::List => {
            if groups.is_empty() {
                println!("No entity groups defined.");
                println!("Create one with: sjbis entity add <name> <member1> <member2> ...");
                return Ok(());
            }
            for (name, val) in groups.iter() {
                let count = val.as_array().map(|a| a.len()).unwrap_or(0);
                println!("  {} ({} member{})", name, count, if count == 1 { "" } else { "s" });
            }
        }
        cli::EntityCommands::Show { name } => {
            let key = name.to_lowercase();
            let found = groups.iter().find(|(k, _)| k.to_lowercase() == key);
            if let Some((actual_name, val)) = found {
                let members: Vec<String> = val.as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                println!("Group: {}", actual_name);
                for m in members {
                    println!("  - {}", m);
                }
            } else {
                anyhow::bail!("Group '{}' not found", name);
            }
        }
        cli::EntityCommands::Rm { name, member } => {
            let key = name.to_lowercase();
            let actual_key = groups.keys().find(|k| k.to_lowercase() == key).cloned();
            if let Some(actual_key) = actual_key {
                if let Some(member_name) = member {
                    // Remove specific member
                    if let Some(arr) = groups.get_mut(&actual_key).and_then(|v| v.as_array_mut()) {
                        let before = arr.len();
                        arr.retain(|v| v.as_str().map_or(true, |s| s.to_lowercase() != member_name.to_lowercase()));
                        let after = arr.len();
                        if after == before {
                            anyhow::bail!("Member '{}' not found in group '{}'", member_name, actual_key);
                        }
                        if arr.is_empty() {
                            groups.remove(&actual_key);
                            println!("Removed last member — deleted group '{}'", actual_key);
                        } else {
                            println!("Removed '{}' from '{}' ({} members remaining)", member_name, actual_key, after);
                        }
                    }
                } else {
                    // Remove entire group
                    groups.remove(&actual_key);
                    println!("Deleted group '{}'", actual_key);
                }
                let content = toml::to_string_pretty(&toml_value)?;
                std::fs::write(&config_path, content)?;
            } else {
                anyhow::bail!("Group '{}' not found", name);
            }
        }
    }
    Ok(())
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

    // Build sub_questions from --form (JSON array or @file)
    let sub_questions = if let Some(ref raw) = args.form {
        let json_str = if raw.starts_with('@') {
            tokio::fs::read_to_string(&raw[1..]).await.ok()
        } else {
            Some(raw.clone())
        };
        json_str.and_then(|s| serde_json::from_str::<Vec<SubQuestion>>(&s).ok())
    } else {
        None
    };

    let req = AskRequest {
        question: unescape(&args.question),
        agent_name: args.agent_name.clone(),
        instance: args.instance.clone(),
        detail: args.detail.as_ref().map(|d| unescape(d)),
        detail_markdown: args.detail_markdown.as_ref().map(|d| unescape(d)),
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
        sub_questions,
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
        // Blocking mode: the daemon returns when answered OR when the
        // --deadline is reached (structured timed_out envelope).
        // Progress goes to stderr so --json stdout stays pure JSON.
        eprintln!("Waiting for answer... (id: {})", notif.id);
        let answer = wait_for_answer(&client, &url, &notif.id).await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&answer)?);
        } else if answer.via == "timed_out" {
            println!("(timed out — no answer before deadline)");
        } else {
            println!("{}", answer.answer.clone().unwrap_or_default());
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
    // The server caps its wait at the notification's deadline (or a 600s
    // ceiling). Use a slightly larger request timeout so the server always
    // returns the structured result (answered OR timed_out) rather than the
    // client aborting first.
    let resp = client
        .get(format!("{}/wait/{}", url, id))
        .timeout(std::time::Duration::from_secs(660))
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

async fn cmd_dismiss(id: String) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/dismiss/{}", url, id)).send().await?;
    if resp.status().is_success() {
        println!("Dismissed {}", id);
    } else {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    Ok(())
}

async fn cmd_answer(id: String, answer: String, via: String, note: Option<String>) -> Result<()> {
    let url = cli::daemon_url(None);
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({ "answer": answer, "via": via });
    if let Some(n) = note {
        body["note"] = serde_json::Value::String(n);
    }
    let resp = client
        .post(format!("{}/answer/{}", url, id))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
    }
    let envelope: AnswerEnvelope = resp.json().await?;
    println!("{}", serde_json::to_string_pretty(&envelope)?);
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
            NotificationStatus::Dismissed => "dismissed",
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
        cli::RuleCommands::Add { text, scope, urgency_min, mute, priority, expires } => {
            let mut body = serde_json::json!({
                "text": text,
                "scope": scope,
                "urgency_min": urgency_min,
                "mute": mute,
                "priority": priority,
            });
            if let Some(exp) = expires {
                body["expires_in"] = serde_json::json!(exp);
            }
            let resp = client.post(format!("{}/rules", url)).json(&body).send().await?;
            if resp.status().is_success() {
                let rules: Vec<Rule> = resp.json().await?;
                for rule in rules {
                    println!("Added rule {}: {}", rule.id, rule.text);
                }
            } else {
                anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
            }
        }
        cli::RuleCommands::Allow { agent, from, for_duration } => {
            let text = format!("allow {} from {} for {}", agent, from, for_duration);
            let body = serde_json::json!({
                "text": text,
                "priority": 0,
            });
            let resp = client.post(format!("{}/rules", url)).json(&body).send().await?;
            if resp.status().is_success() {
                let rules: Vec<Rule> = resp.json().await?;
                println!("Created {} rule(s) for allow-list:", rules.len());
                for rule in &rules {
                    let expires = rule.expires_at.map(|e| format!(" (expires {})", e)).unwrap_or_default();
                    let pri = if rule.priority > 0 { format!(" [pri:{}]", rule.priority) } else { String::new() };
                    println!("  [{}] {}{} — {}", rule.id, rule.text, pri, expires);
                }
            } else {
                anyhow::bail!("daemon error: {}", resp.text().await.unwrap_or_default());
            }
        }
        cli::RuleCommands::List => {
            let resp = client.get(format!("{}/state", url)).send().await?;
            let state: DashboardState = resp.json().await?;
            let now = chrono::Utc::now();
            for r in state.rules {
                let status = if !r.active {
                    "inactive"
                } else if let Some(exp) = r.expires_at {
                    if now > exp {
                        "expired"
                    } else {
                        let mins = (exp - now).num_minutes();
                        &format!("active (~{}m left)", mins)
                    }
                } else {
                    "active"
                };
                let pri = if r.priority > 0 { format!(" [pri:{}]", r.priority) } else { String::new() };
                println!("[{}] {}{} — {}", r.id, status, pri, r.text);
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

/// Spawn a detached background daemon process on the given port.
/// Writes the pidfile and returns the child pid.
fn spawn_background_daemon(port: u16) -> Result<u32> {
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
    Ok(pid)
}

async fn cmd_daemon(command: cli::DaemonCommands) -> Result<()> {
    match command {
        cli::DaemonCommands::Start { port, background } => {
            if background {
                let pid = spawn_background_daemon(port)?;
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

    let health_ok = |url: &str| {
        let client = client.clone();
        let url = url.to_string();
        async move {
            matches!(
                client.get(format!("{}/health", url)).timeout(std::time::Duration::from_secs(2)).send().await,
                Ok(resp) if resp.status().is_success()
            )
        }
    };

    let mut daemon_ok = health_ok(&url).await;

    // If the daemon isn't responding AND it's a local URL, auto-start it.
    // Remote daemons (e.g. on another host) can't be started from here.
    let is_local = url.contains("localhost") || url.contains("127.0.0.1") || url.contains("0.0.0.0");
    let mut autostart_note = String::new();
    if !daemon_ok && is_local {
        let port = url.rsplit(':').next().and_then(|s| s.trim_end_matches('/').parse::<u16>().ok()).unwrap_or(7878);
        match spawn_background_daemon(port) {
            Ok(pid) => {
                // Give it a moment to bind and run migrations.
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    if health_ok(&url).await { break; }
                }
                daemon_ok = health_ok(&url).await;
                autostart_note = if daemon_ok {
                    format!(" (auto-started, pid {})", pid)
                } else {
                    format!(" (auto-start attempted, pid {}, still coming up)", pid)
                };
            }
            Err(e) => {
                autostart_note = format!(" (auto-start failed: {})", e);
            }
        }
    }

    let status_banner = if daemon_ok {
        format!("  Daemon: running on {}{}", url, autostart_note)
    } else if is_local {
        format!("  Daemon: NOT RUNNING on {}{} — start manually with: sjbis daemon start --port 7878", url, autostart_note)
    } else {
        format!("  Daemon: UNREACHABLE at {} (remote — start it on that host)", url)
    };

    let help_body = r#"  The surfacer is a dashboard where humans see and answer questions
  from agents (scripts, tools, AI systems). Agents post questions via
  the CLI. The daemon must be running first.

DEFAULT WORKING AGREEMENT FOR QUESTIONS  (how YOU, the agent, must behave)
  When you have a question for Stephen, ask it as a BLOCKING question
  with a deadline. The daemon honors the deadline: it returns as soon as
  Stephen answers, or returns a timed_out result when the deadline hits.
  You do NOT need to build your own wait/poll loop.

  Copy-paste pattern (1-minute deadline; adjust as needed):

    # 1. Ask, blocking, with a deadline. --json gives a structured result.
    res=$(sjbis ask --question "..." --choices "a,b,c" \
            --agent-name <you> --deadline 1m --blocking --json)

    # 2. Read the result. `via` is "timed_out" if the deadline passed
    #    with no answer; otherwise `answer` holds Stephen's choice.
    ans=$(echo "$res" | python3 -c 'import sys,json
d=json.load(sys.stdin)
print("" if d.get("via")=="timed_out" else (d.get("answer") or ""))')

    # 3. If Stephen answered, follow his answer.
    # 4. If it timed out (ans is empty), DO NOT give up — proceed with
    #    your BEST JUDGEMENT, then INFORM the server so the dashboard
    #    shows the auto-pick (lingers ~10s, marked "agent auto-picked"):
    if [ -z "$ans" ]; then
      id=$(echo "$res" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
      ans="<your sensible default>"
      sjbis answer "$id" --answer "$ans" --via caller-timeout \
        --note "No reply in time — went with <your default> because <why>."
    fi

  Notes:
    * --blocking + --deadline is the intended flow; the daemon caps the
      wait at the deadline, so the command returns promptly. No manual
      polling needed.
    * Always pass --json so you can read `via` (answered vs timed_out)
      and `id` (needed to record an auto-pick).
    * For a chain of path-dependent questions, repeat this per question,
      branching on the previous answer. Ask ONE question at a time.
    * `--via caller-timeout` is what makes the dashboard render your
      decision as an agent auto-pick rather than a human answer.

  This keeps automated workflows moving: the human gets the deadline to
  weigh in, and silence is treated as "use your judgement," not a hang.

DAEMON
  `sjbis prime` auto-starts a LOCAL daemon if one isn't already running
  (see the status line at the top of this output). A remote daemon must
  be started on its own host. Manual control / deployment (systemd, etc.)
  is documented in the project README.

POSTING A QUESTION (fire-and-forget)
  sjbis ask --question "Deploy to prod?" --yesno --agent-name deploybot
  sjbis ask --question "Lunch cuisine?" --choices "thai,indian,salad" --agent-name lunchbot

POSTING A QUESTION (synchronous / blocking)
  Add --blocking to wait for the human answer. With --deadline set, the
  daemon caps the wait at the deadline and returns a structured
  timed_out result if no answer arrives — so the command returns
  promptly, not after a long hang.

  sjbis ask --question "Approve PR #412?" --yesno --blocking \
    --deadline 1m --agent-name codebot

  The answer is printed to stdout when it arrives. Use --json for
  structured output: check `via` ("timed_out" vs a real answer),
  plus latency_ms, answer_label, etc. On timeout, follow the DEFAULT
  WORKING AGREEMENT above (apply best judgement, then `sjbis answer
  <id> --via caller-timeout` to record it).

  Without --deadline, a blocking wait can stay open up to ~10 minutes.

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

FORMATTING
  --question and --detail support escape sequences:
    \\n  New line
    \\t  Tab
    \\\\  Literal backslash

  --detail-markdown renders rich text (bold **text**, italic *text*,
  lists - item, links [text](url), code `inline`, headings ### h3).

  Example:
    sjbis ask --question "Deploy?" --detail "Context:\\n- Staging passed\\n- Prod is idle" \\n      --yesno --agent-name deploybot

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

ENTITY GROUPS (named contact lists for rules)
  Define reusable groups in ~/.config/sjbis/entities.toml:
    [groups]
    family = ["Jeff", "Carmen", "Mom"]
    work   = ["boss", "team-lead"]

  sjbis entity list             Show all groups
  sjbis entity show family      Show members of a group
  sjbis entity add family Jeff Carmen Mom
  sjbis entity rm family --member Jeff
  sjbis entity rm family        Delete entire group

RULES (filtering / muting / allow-lists)
  Rules are evaluated in priority order (highest applied last, can override).
  Time-bounded rules auto-expire and disappear from the active list.

  sjbis rule list               Show active rules with time remaining
  sjbis rule rm r-abc123        Remove a rule by id

  Natural language — no syntax to memorize. The compiler handles:
    "mute all iMessage"                     → mutes everything from iMessage
    "mute everyone"                       → mutes all agents (global DND)
    "mute iMessage except family for 1h"  → only family gets through
    "only allow Signal from family"         → blocks all Signal except family
    "urgent only"                         → only urgency 4+ surfaces
    "auto-ack iMessage"                     → dismiss without answering
    "reprioritize iMessage to urgent"     → bump urgency to 4

  Entity groups expand automatically — "family" becomes Jeff, Carmen, Mom, Dad.
  The simple compiler handles common patterns instantly (offline, zero latency).
  For novel phrasing the AI compiler falls back to the LLM if FIREWORKS_API_KEY is set.

  Explicit allow-list (same as "only allow ..."):
    sjbis rule allow --agent iMessage --from "Jeff,Carmen,JCS-Central" --for-duration 1h

LIST / STATUS / CANCEL / DISMISS
  sjbis list                    Show open notifications
  sjbis status sjbis-AbCdEfGh   Check state of any notification (open/answered/cancelled/timed_out/dismissed)
  sjbis cancel sjbis-AbCdEfGh   Cancel an open notification
  sjbis dismiss sjbis-AbCdEfGh  Dismiss (mark as seen without answering — no reply sent)

UPGRADE
  sjbis upgrade --check         See whether a newer release is available on GitHub
  sjbis upgrade                 Download the latest release and replace this binary
  sjbis upgrade --tag v0.1.2    Install a specific tagged release
"#;
    println!("SJBIS — How to ask questions  (cli {})\n\n{}\n\n{}", version::full(), status_banner, help_body);
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

async fn cmd_upgrade(check: bool, force: bool, tag: Option<String>) -> Result<()> {
    upgrade::run(check, force, tag).await
}
