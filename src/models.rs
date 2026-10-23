use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
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
    #[serde(rename = "form")]
    Form,
    /// Transient-authority approval: can only be answered by a signed grant
    /// (POST /grants), never by a plain POST /answer.
    #[serde(rename = "approval")]
    Approval,
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
            QuestionType::Form => "form",
            QuestionType::Approval => "approval",
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
            "form" => Ok(QuestionType::Form),
            "approval" => Ok(QuestionType::Approval),
            _ => Err(format!("unknown question type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl<'de> Deserialize<'de> for Choice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CanonicalChoice {
            value: String,
            label: String,
            #[serde(default)]
            hint: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ChoiceInput {
            String(String),
            Canonical(CanonicalChoice),
        }

        Ok(match ChoiceInput::deserialize(deserializer)? {
            ChoiceInput::String(value) => Self {
                label: value.clone(),
                value,
                hint: None,
            },
            ChoiceInput::Canonical(choice) => Self {
                value: choice.value,
                label: choice.label,
                hint: choice.hint,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuestion {
    pub key: String,
    pub question: String,
    pub shape: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "default")]
    pub default_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_label: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_questions: Option<Vec<SubQuestion>>,
    /// Requested lifetime (seconds) of the grant an approval ask asks for.
    /// Defaults to [`DEFAULT_APPROVAL_TTL_SECS`] for approval asks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<i64>,
    /// Reauth: the ta grant id this approval ask re-proposes. A signing
    /// device copies it into the signed record's `renews`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renews: Option<String>,
    /// Single-use grant requested (approval asks only). A signing device
    /// copies it into the signed record's `once`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

/// Default requested grant lifetime for an approval ask (1 hour).
pub const DEFAULT_APPROVAL_TTL_SECS: i64 = 3600;

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
    /// Signed ta grant envelope, present only when an approval ask was
    /// answered by a signed grant. Nothing sets this yet (see the grants bead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<serde_json::Value>,
    /// The ask's single-use flag (approval asks that set `once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
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

/// Agent identity plus notification-derived activity metadata for API reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSummary {
    pub name: String,
    pub glyph: String,
    pub color: String,
    pub kind: String,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub has_open_notification: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_questions: Option<Vec<SubQuestion>>,
    /// Requested grant lifetime in seconds (approval asks). Must be > 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<i64>,
    /// Reauth (ta): the grant id this ask re-proposes. Stored verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renews: Option<String>,
    /// Request a single-use grant (approval asks only; `true` on any other
    /// question type is a 422).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
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
    pub agents: HashMap<String, SourceSummary>,
    #[serde(default)]
    pub version: String,
}

/// Parsed deadline string: either duration ("6m", "2h") or absolute timestamp
pub fn parse_deadline(s: &str) -> anyhow::Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    Ok(Utc::now() + parse_duration(s)?)
}

/// Parse a relative duration like "90s", "6m", "2h", "1d" or "500ms".
/// Shared by `--deadline` (relative form) and `--ttl`.
pub fn parse_duration(s: &str) -> anyhow::Result<chrono::Duration> {
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
                    return Ok(dur2);
                }
            }
            anyhow::bail!("unknown deadline format: {}", s)
        }
    };
    Ok(dur)
}

/// Resolve and validate the requested grant TTL for an ask.
///
/// Any explicit `ttl_secs` must be strictly positive. Approval asks default to
/// [`DEFAULT_APPROVAL_TTL_SECS`] when no TTL is given; other question types
/// keep whatever (validated) value the caller sent.
pub fn resolve_ttl_secs(
    question_type: &QuestionType,
    ttl_secs: Option<i64>,
) -> Result<Option<i64>, String> {
    match ttl_secs {
        Some(t) if t <= 0 => Err(format!("ttl_secs must be > 0 (got {t})")),
        Some(t) => Ok(Some(t)),
        None if *question_type == QuestionType::Approval => Ok(Some(DEFAULT_APPROVAL_TTL_SECS)),
        None => Ok(None),
    }
}

/// Resolve the single-use flag for an ask.
///
/// `once` is only meaningful on approval asks: `once: true` on any other
/// question type is an error, `once: false` there is dropped, and an approval
/// ask keeps whatever the caller sent (absent means reusable).
pub fn resolve_once(
    question_type: &QuestionType,
    once: Option<bool>,
) -> Result<Option<bool>, String> {
    match (question_type, once) {
        (QuestionType::Approval, once) => Ok(once),
        (_, Some(true)) => Err(format!(
            "once is only valid on approval asks (question_type is {question_type})"
        )),
        (_, _) => Ok(None),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ask_request_normalizes_mixed_choice_inputs() {
        let request: AskRequest = serde_json::from_value(json!({
            "question": "Which fixes should run?",
            "question_type": "multichoice",
            "choices": [
                "Run formatter",
                {
                    "value": "tests",
                    "label": "Run targeted tests",
                    "hint": "Recommended"
                }
            ],
            "sub_questions": [{
                "key": "follow_up",
                "question": "Then what?",
                "shape": "multichoice",
                "choices": ["Open PR"]
            }]
        }))
        .expect("the producer wire union should deserialize");

        assert_eq!(
            request.choices,
            Some(vec![
                Choice {
                    value: "Run formatter".to_string(),
                    label: "Run formatter".to_string(),
                    hint: None,
                },
                Choice {
                    value: "tests".to_string(),
                    label: "Run targeted tests".to_string(),
                    hint: Some("Recommended".to_string()),
                },
            ])
        );
        let sub_question = request
            .sub_questions
            .expect("sub-question should be present")
            .into_iter()
            .next()
            .expect("one sub-question should be present");
        assert_eq!(
            sub_question.choices,
            Some(vec![Choice {
                value: "Open PR".to_string(),
                label: "Open PR".to_string(),
                hint: None,
            }])
        );
    }

    #[test]
    fn choices_serialize_and_storage_round_trip_as_canonical_objects() {
        let legacy_jsonb = json!(["first", {"value": "second", "label": "Second"}]);
        let choices: Vec<Choice> =
            serde_json::from_value(legacy_jsonb).expect("legacy string arrays should load");

        let stored = serde_json::to_value(&choices).expect("choices should serialize");
        assert_eq!(
            stored,
            json!([
                {"value": "first", "label": "first"},
                {"value": "second", "label": "Second"}
            ])
        );
        assert_eq!(
            serde_json::from_value::<Vec<Choice>>(stored).expect("canonical choices should reload"),
            choices
        );
    }

    #[test]
    fn question_type_approval_round_trips() {
        let qt: QuestionType =
            serde_json::from_value(json!("approval")).expect("approval should deserialize");
        assert_eq!(qt, QuestionType::Approval);
        assert_eq!(serde_json::to_value(&qt).unwrap(), json!("approval"));
        assert_eq!(qt.to_string(), "approval");
        assert_eq!(
            "approval".parse::<QuestionType>(),
            Ok(QuestionType::Approval)
        );
    }

    #[test]
    fn ask_request_accepts_approval_with_ttl_and_choices() {
        let request: AskRequest = serde_json::from_value(json!({
            "question": "Do you want to fire the missiles?",
            "question_type": "approval",
            "ttl_secs": 600,
            "choices": ["plan A", "plan B"]
        }))
        .expect("approval ask should deserialize");
        assert_eq!(request.question_type, Some(QuestionType::Approval));
        assert_eq!(request.ttl_secs, Some(600));
        assert_eq!(request.choices.map(|c| c.len()), Some(2));
    }

    #[test]
    fn renews_round_trips_through_ask_and_notification() {
        let request: AskRequest = serde_json::from_value(json!({
            "question": "Do you want to fire the missiles?",
            "question_type": "approval",
            "renews": "ta-3kp9ab"
        }))
        .expect("reauth ask should deserialize");
        assert_eq!(request.renews.as_deref(), Some("ta-3kp9ab"));

        let mut notif: Notification = serde_json::from_value(json!({
            "id": "sjbis-x", "agent_name": "a", "sender": "a", "src": "a",
            "question": "q", "question_type": "approval", "urgency": 2,
            "blocking": true, "deadline": null, "reply_to": "stdout",
            "status": "open", "created_at": "2026-10-23T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(notif.renews, None);
        assert!(
            serde_json::to_value(&notif)
                .unwrap()
                .get("renews")
                .is_none()
        );
        notif.renews = request.renews;
        assert_eq!(serde_json::to_value(&notif).unwrap()["renews"], "ta-3kp9ab");
    }

    #[test]
    fn ask_request_accepts_once_and_omits_it_when_absent() {
        let request: AskRequest = serde_json::from_value(json!({
            "question": "Do you want to fire the missiles?",
            "question_type": "approval",
            "ttl_secs": 600,
            "once": true
        }))
        .expect("single-use approval ask should deserialize");
        assert_eq!(request.once, Some(true));
        assert_eq!(serde_json::to_value(&request).unwrap()["once"], true);

        let request: AskRequest = serde_json::from_value(json!({
            "question": "q", "question_type": "approval"
        }))
        .unwrap();
        assert_eq!(request.once, None);
        assert!(
            serde_json::to_value(&request)
                .unwrap()
                .get("once")
                .is_none()
        );
    }

    #[test]
    fn resolve_once_is_approval_only() {
        assert_eq!(
            resolve_once(&QuestionType::Approval, Some(true)),
            Ok(Some(true))
        );
        assert_eq!(
            resolve_once(&QuestionType::Approval, Some(false)),
            Ok(Some(false))
        );
        assert_eq!(resolve_once(&QuestionType::Approval, None), Ok(None));
        assert!(resolve_once(&QuestionType::YesNo, Some(true)).is_err());
        assert_eq!(resolve_once(&QuestionType::YesNo, Some(false)), Ok(None));
        assert_eq!(resolve_once(&QuestionType::FreeText, None), Ok(None));
    }

    #[test]
    fn resolve_ttl_secs_defaults_and_validates() {
        assert_eq!(
            resolve_ttl_secs(&QuestionType::Approval, None),
            Ok(Some(DEFAULT_APPROVAL_TTL_SECS))
        );
        assert_eq!(
            resolve_ttl_secs(&QuestionType::Approval, Some(60)),
            Ok(Some(60))
        );
        assert_eq!(resolve_ttl_secs(&QuestionType::YesNo, None), Ok(None));
        for bad in [0, -1, i64::MIN] {
            assert!(resolve_ttl_secs(&QuestionType::Approval, Some(bad)).is_err());
            assert!(resolve_ttl_secs(&QuestionType::YesNo, Some(bad)).is_err());
        }
    }

    #[test]
    fn parse_duration_matches_deadline_units() {
        assert_eq!(
            parse_duration("90s").unwrap(),
            chrono::Duration::seconds(90)
        );
        assert_eq!(parse_duration("6m").unwrap(), chrono::Duration::minutes(6));
        assert_eq!(parse_duration("1h").unwrap(), chrono::Duration::hours(1));
        assert_eq!(parse_duration("2d").unwrap(), chrono::Duration::days(2));
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("1").is_err());
    }

    #[test]
    fn answer_envelope_omits_grant_when_absent() {
        let mut envelope = AnswerEnvelope {
            id: "sjbis-x".to_string(),
            answer: None,
            answer_label: None,
            answered_at: None,
            latency_ms: None,
            renderer: "approval".to_string(),
            src: "demo".to_string(),
            via: "timed_out".to_string(),
            note: None,
            grant: None,
            once: None,
        };
        let value = serde_json::to_value(&envelope).unwrap();
        assert!(value.get("grant").is_none());
        assert!(value.get("once").is_none());

        envelope.grant = Some(json!({"id": "ta-7f3k2q"}));
        let value = serde_json::to_value(&envelope).unwrap();
        assert_eq!(value["grant"]["id"], "ta-7f3k2q");

        envelope.once = Some(true);
        assert_eq!(serde_json::to_value(&envelope).unwrap()["once"], true);
    }

    #[test]
    fn ask_request_rejects_non_choice_entries() {
        for invalid_choice in [
            json!(42),
            json!(["nested"]),
            json!({"value": "missing-label"}),
            json!({"value": "x", "label": "X", "extra": true}),
        ] {
            let result = serde_json::from_value::<AskRequest>(json!({
                "question": "Choose",
                "question_type": "multichoice",
                "choices": [invalid_choice]
            }));
            assert!(result.is_err(), "invalid choice entry should fail ingestion");
        }
    }
}
