use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "sjbis")]
#[command(about = "SJBIS — Stephen J Barr Information Surfacer")]
#[command(version = "0.1.0")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Post a new question to the surfacer
    Ask(AskArgs),
    /// List open notifications
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Cancel a notification by id
    Cancel {
        /// Notification id
        id: String,
    },
    /// Dismiss a notification by id (mark as seen without answering)
    Dismiss {
        /// Notification id
        id: String,
    },
    /// Answer a question on behalf of the caller (e.g. after a timeout,
    /// the agent proceeds with its best judgement and informs the server)
    Answer {
        /// Notification id
        id: String,
        /// The answer value to record
        #[arg(long)]
        answer: String,
        /// How the answer was produced (default: caller)
        #[arg(long, default_value = "caller")]
        via: String,
        /// Optional free-text note shown on the dashboard and returned to the caller
        #[arg(long)]
        note: Option<String>,
    },
    /// Wait for an answer to a previously-posted question
    Wait {
        /// Notification id
        id: String,
    },
    /// Get the current status of a notification by id
    Status {
        /// Notification id
        id: String,
    },
    /// Manage rules
    Rule {
        #[command(subcommand)]
        command: RuleCommands,
    },
    /// Manage entity groups (named contact lists for rules)
    Entity {
        #[command(subcommand)]
        command: EntityCommands,
    },
    /// Daemon lifecycle
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
    /// Show how to ask questions (primer for agents)
    Prime,
    /// Register a long-running agent identity
    Register {
        #[arg(long)]
        agent_name: String,
        #[arg(long)]
        glyph: Option<String>,
        #[arg(long)]
        color: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RuleCommands {
    Add {
        /// Rule text (natural language)
        text: String,
        /// Scope (agent name or *)
        #[arg(long)]
        scope: Option<String>,
        /// Minimum urgency to match
        #[arg(long, default_value = "0")]
        urgency_min: i32,
        /// Mute matching notifications
        #[arg(long)]
        mute: bool,
        /// Rule priority (higher = applied later, can override lower)
        #[arg(long, default_value = "0")]
        priority: i32,
        /// Auto-expire after duration (e.g. 1h, 30m, 1d)
        #[arg(long)]
        expires: Option<String>,
    },
    /// Allow messages from specific contacts only, mute everything else
    Allow {
        /// Agent name (e.g. iMessage, Signal)
        #[arg(long)]
        agent: String,
        /// Comma-separated list of contacts to allow
        #[arg(long)]
        from: String,
        /// Duration to keep rule active (e.g. 1h, 30m)
        #[arg(long, default_value = "1h")]
        for_duration: String,
    },
    List,
    Rm {
        /// Rule id to remove
        id: String,
    },
}

#[derive(Subcommand)]
pub enum EntityCommands {
    /// Create or replace an entity group
    Add {
        /// Group name (e.g. family, work_team)
        name: String,
        /// Members (space-separated names, numbers, or handles)
        members: Vec<String>,
    },
    /// List all entity groups
    List,
    /// Show members of a specific group
    Show {
        /// Group name
        name: String,
    },
    /// Remove a group or a member from a group
    Rm {
        /// Group name
        name: String,
        /// Optional member to remove (omit to delete entire group)
        #[arg(long)]
        member: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DaemonCommands {
    Start {
        /// Port to listen on
        #[arg(short, long, default_value = "7878")]
        port: u16,
        /// Background (detach)
        #[arg(short, long)]
        background: bool,
    },
    Stop,
    Status,
}

#[derive(Parser, Clone)]
pub struct AskArgs {
    /// Question text (max 280 chars recommended)
    #[arg(short, long)]
    pub question: String,
    /// Extra context paragraph
    #[arg(long)]
    pub detail: Option<String>,
    /// Extra context as markdown (renders bold, lists, links, etc.)
    #[arg(long)]
    pub detail_markdown: Option<String>,
    /// Answer shape: yes/no
    #[arg(long)]
    pub yesno: bool,
    /// Answer shape: multi-choice (JSON array or CSV)
    #[arg(long)]
    pub choices: Option<String>,
    /// Answer shape: free text
    #[arg(long)]
    pub text: bool,
    /// Answer shape: numeric
    #[arg(long)]
    pub number: bool,
    /// Answer shape: file upload
    #[arg(long)]
    pub file: bool,
    /// Answer shape: approve/reject a diff
    #[arg(long)]
    pub diff: bool,
    /// Answer shape: acknowledge only
    #[arg(long)]
    pub ack: bool,
    /// Answer shape: pick from a list
    #[arg(long)]
    pub pick: Option<String>,
    /// Answer shape: schedule slot
    #[arg(long)]
    pub schedule: Option<String>,
    /// Let the AI guess the renderer
    #[arg(long)]
    pub guess_renderer: bool,
    /// Calling agent name (required)
    #[arg(long)]
    pub agent_name: String,
    /// Per-invocation instance detail
    #[arg(long)]
    pub instance: Option<String>,
    /// Urgency 0-5 (default 2)
    #[arg(short, long, default_value = "2")]
    pub urgency: i32,
    /// Caller blocks until answer
    #[arg(short, long)]
    pub blocking: bool,
    /// Deadline: duration (90s, 6m, 2h) or ISO timestamp
    #[arg(short, long)]
    pub deadline: Option<String>,
    /// Reply channel: stdout | webhook:URL | file:PATH | exit-code
    #[arg(long)]
    pub reply_to: Option<String>,
    /// Idempotency key (24h dedupe window)
    #[arg(long)]
    pub id: Option<String>,
    /// Output structured JSON
    #[arg(long)]
    pub json: bool,
    /// Custom yes label
    #[arg(long)]
    pub yes_label: Option<String>,
    /// Custom no label
    #[arg(long)]
    pub no_label: Option<String>,
    /// Placeholder for text input
    #[arg(long)]
    pub placeholder: Option<String>,
    /// Suggestions for text input (newline-separated)
    #[arg(long)]
    pub suggestions: Option<String>,
    /// Min value for numeric
    #[arg(long)]
    pub min: Option<f64>,
    /// Max value for numeric
    #[arg(long)]
    pub max: Option<f64>,
    /// Step for numeric
    #[arg(long, default_value = "1")]
    pub step: f64,
    /// Default value for numeric
    #[arg(long)]
    pub default: Option<f64>,
    /// Unit label for numeric
    #[arg(long)]
    pub unit: Option<String>,
    /// Accepted file extensions for file type
    #[arg(long)]
    pub accept: Option<String>,
    /// Mute key for coalescing
    #[arg(long)]
    pub mute_key: Option<String>,
    /// Privacy mode: public | redact-pii | private
    #[arg(long)]
    pub privacy: Option<String>,
    /// Daemon URL override
    #[arg(long)]
    pub daemon: Option<String>,
}

impl AskArgs {
    pub fn question_type(&self) -> Option<crate::models::QuestionType> {
        use crate::models::QuestionType;
        if self.yesno { return Some(QuestionType::YesNo); }
        if self.choices.is_some() { return Some(QuestionType::Multichoice); }
        if self.text { return Some(QuestionType::FreeText); }
        if self.number { return Some(QuestionType::Numeric); }
        if self.file { return Some(QuestionType::File); }
        if self.diff { return Some(QuestionType::Diff); }
        if self.ack { return Some(QuestionType::Ack); }
        if self.pick.is_some() { return Some(QuestionType::PickList); }
        if self.schedule.is_some() { return Some(QuestionType::Schedule); }
        if self.guess_renderer { return None; }
        // Default: if no shape given and not guessing, treat as ack
        Some(QuestionType::Ack)
    }
}

/// Load daemon URL from ~/.config/sjbis/daemon.toml (or platform config dir)
pub fn load_daemon_url() -> Option<String> {
    let candidates = [
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
            .join(".config/sjbis/daemon.toml"),
        dirs::config_dir()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("sjbis/daemon.toml"),
    ];

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(config) = toml::from_str::<toml::Value>(&content) {
                if let Some(url) = config.get("url").and_then(|v| v.as_str()) {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

/// Resolve the daemon base URL from arg, env, config file, or default
pub fn daemon_url(arg: Option<String>) -> String {
    arg.or_else(|| std::env::var("SJBIS_DAEMON").ok())
        .or_else(load_daemon_url)
        .unwrap_or_else(|| "http://localhost:7878".to_string())
}

/// Path to the pidfile
pub fn pidfile_path() -> PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(|| std::env::temp_dir()).join("sjbis");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("daemon.pid")
}

/// Load Postgres DSN from ~/.config/sjbis/database.toml (or platform config dir)
pub fn load_dsn() -> anyhow::Result<String> {
    let candidates = [
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
            .join(".config/sjbis/database.toml"),
        dirs::config_dir()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("sjbis/database.toml"),
    ];

    let mut last_err = None;
    for path in &candidates {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let config: toml::Value = toml::from_str(&content)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                let dsn = config.get("database")
                    .and_then(|d: &toml::Value| d.get("dsn"))
                    .and_then(|v: &toml::Value| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("database.toml missing [database] dsn field"))?;
                return Ok(dsn.to_string());
            }
            Err(e) => last_err = Some((path.clone(), e)),
        }
    }
    let (path, e) = last_err.unwrap();
    Err(anyhow::anyhow!("failed to read database config at {}: {}", path.display(), e))
}
