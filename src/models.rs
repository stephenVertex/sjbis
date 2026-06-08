use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique notification id: sjbis-{nanoid}
pub type NotificationId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QuestionType {
    #[serde(rename = "yesno")]
    YesNo,
    #[serde(rename = "multichoice")]
    Multichoice,
    #[serde(rename = "freetext")]
    FreeText,
    #[serde(rename = "numeric")]
    Numeric,
    #[serde(rename = "file")]
    File,
    #[serde(rename = "diff")]
    Diff,
    #[serde(rename = "ack")]
    Ack,
    #[serde(rename = "picklist")]
    PickList,
    #[serde(rename = "schedule")]
    Schedule,
}

impl std::fmt::Display for QuestionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            QuestionType::YesNo => "yesno",
            QuestionType::Multichoice => "multichoice",
            QuestionType::FreeText => "freetext",
            QuestionType::Numeric => "numeric",
            QuestionType::File => "file",
            QuestionType::Diff => "diff",
            QuestionType::Ack => "ack",
            QuestionType::PickList => "picklist",
            QuestionType::Schedule => "schedule",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for QuestionType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "yesno" => Ok(QuestionType::YesNo),
            "multichoice" => Ok(QuestionType::Multichoice),
            "freetext" => Ok(QuestionType::FreeText),
            "numeric" => Ok(QuestionType::Numeric),
            "file" => Ok(QuestionType::File),
            "diff" => Ok(QuestionType::Diff),
            "ack" => Ok(QuestionType::Ack),
            "picklist" => Ok(QuestionType::PickList),
            "schedule" => Ok(QuestionType::Schedule),
            _ => Err(format!("unknown question type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub day: String,
    pub time: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: String, // meta, add, del, ctx
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickItem {
    pub id: String,
    pub title: String,
    pub meta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyTo {
    Stdout,
    Webhook { url: String },
    File { path: String },
    ExitCode,
}

impl Default for ReplyTo {
    fn default() -> Self {
        ReplyTo::Stdout
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: NotificationId,
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub sender: String,
    pub src: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_markdown: Option<String>,
    pub question_type: QuestionType,
    pub urgency: i32, // 0..5
    pub blocking: bool,
    pub deadline: Option<DateTime<Utc>>,
    pub reply_to: ReplyTo,
    pub status: NotificationStatus,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_label: Option<String>,
    // Type-specific fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<DiffLine>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<PickItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<Slot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_id: Option<String>, // idempotency key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snooze_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    Open,
    Answered,
    Cancelled,
    Muted,
    TimedOut,
    Dismissed,
}

/// The envelope returned to a blocking caller or webhook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerEnvelope {
    pub id: NotificationId,
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,
    pub renderer: String,
    pub src: String,
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Rule compiled from natural language or written directly as JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled: Option<serde_json::Value>,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub urgency_min: i32,
    #[serde(default)]
    pub mute: bool,
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_window: Option<ActiveWindow>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveWindow {
    pub start: String, // HH:MM
    pub end: String,   // HH:MM
    pub tz: String,
}

/// Agent identity: deterministic glyph + color
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub glyph: String,
    pub color: String,
    pub kind: String,
}

/// New notification request from a caller
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskRequest {
    pub question: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_markdown: Option<String>,
    #[serde(default)]
    pub urgency: i32,
    #[serde(default)]
    pub blocking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>, // caller idempotency key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_type: Option<QuestionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<DiffLine>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<PickItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<Slot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<String>,
}

/// Answer submitted by the user via dashboard
#[derive(Debug, Clone, Deserialize)]
pub struct AnswerRequest {
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Router response from AI
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgency_predicted: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renderer_suggested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub looks_like_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub renderer_guessed: bool,
}

/// The payload pushed over SSE to dashboard clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SseEvent {
    NotificationCreated { notification: Notification },
    NotificationUpdated { notification: Notification },
    NotificationAnswered { envelope: AnswerEnvelope },
    NotificationCancelled { id: NotificationId },
    NotificationDismissed { id: NotificationId, envelope: AnswerEnvelope },
    RuleCreated { rule: Rule },
    RuleUpdated { rule: Rule },
    RuleDeleted { id: String },
}

/// Dashboard init payload: everything the UI needs on first load
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardState {
    pub notifications: Vec<Notification>,
    pub history: Vec<Notification>,
    pub rules: Vec<Rule>,
    pub agents: HashMap<String, Agent>,
    #[serde(default)]
    pub version: String,
}

/// Parsed deadline string: either duration ("6m", "2h") or absolute timestamp
pub fn parse_deadline(s: &str) -> anyhow::Result<DateTime<Utc>> {
    let now = Utc::now();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try parsing durations like "90s", "6m", "2h"
    let s = s.trim();
    if s.len() < 2 {
        anyhow::bail!("invalid deadline format: {}", s);
    }
    let (num_part, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_part.parse()?;
    let dur = match unit {
        "s" => chrono::Duration::seconds(num),
        "m" => chrono::Duration::minutes(num),
        "h" => chrono::Duration::hours(num),
        "d" => chrono::Duration::days(num),
        _ => {
            // Maybe the whole thing is a number followed by unit like "6m"
            // Try with last 2 chars
            if s.len() >= 3 {
                let (num_part2, unit2) = s.split_at(s.len() - 2);
                if let Ok(num2) = num_part2.parse::<i64>() {
                    let dur2 = match unit2 {
                        "ms" => chrono::Duration::milliseconds(num2),
                        _ => anyhow::bail!("unknown deadline unit: {}", unit2),
                    };
                    return Ok(now + dur2);
                }
            }
            anyhow::bail!("unknown deadline format: {}", s)
        }
    };
    Ok(now + dur)
}

/// Generate a deterministic, visually-distinct OKLCH color from an agent name.
///
/// Uses a curated palette of well-separated hues (rather than a raw
/// `hash % 360`, which can map different agents to near-identical hues) and
/// nudges lightness/chroma per slot so adjacent cards read as clearly
/// different colors. Deterministic: the same name always yields the same color.
pub fn agent_color(name: &str) -> String {
    // Hand-tuned hue stops spread around the wheel for maximum separation
    // (lime, teal, blue, violet, magenta, red, orange, amber, green, cyan).
    const HUES: [f64; 10] = [
        130.0, 175.0, 245.0, 295.0, 330.0, 25.0, 55.0, 90.0, 150.0, 210.0,
    ];
    // Slight lightness offsets so two agents that land in nearby hues still
    // differ in tone.
    const LIGHTS: [f64; 3] = [72.0, 78.0, 84.0];

    let mut h = 0u32;
    for c in name.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    let hue = HUES[(h as usize) % HUES.len()];
    // Use a different part of the hash for lightness so it's decorrelated from hue.
    let light = LIGHTS[((h >> 8) as usize) % LIGHTS.len()];
    // Higher chroma than before for more saturated, distinguishable cards.
    format!("oklch({}% 0.21 {})", light, hue)
}

/// Generate a short notification id
pub fn generate_id() -> String {
    format!("sjbis-{}", nanoid::nanoid!(8))
}
