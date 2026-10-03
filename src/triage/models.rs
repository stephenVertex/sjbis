use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

pub const DEFAULT_STRIP_SUFFIX: &str = "_analysis.md";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSpec {
    Glob { patterns: Vec<String> },
    JsonList { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub markdown: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
}

impl Observation {
    pub fn source_kind(&self) -> SourceKind {
        if self.path.is_some() {
            SourceKind::Path
        } else {
            SourceKind::Inline
        }
    }

    pub fn content_sha256(&self) -> String {
        sha256_hex(self.markdown.as_bytes())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateQueue {
    pub name: String,
    pub root: String,
    pub strip_suffix: String,
    pub source_spec: SourceSpec,
    pub items: Vec<Observation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueStatus {
    Open,
    Closed,
}

impl QueueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

impl FromStr for QueueStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            _ => Err(format!("unknown triage queue status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Path,
    Inline,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Inline => "inline",
        }
    }
}

impl FromStr for SourceKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "path" => Ok(Self::Path),
            "inline" => Ok(Self::Inline),
            _ => Err(format!("unknown triage source kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    Current,
    Missing,
    ContentChanged,
    Ambiguous,
}

impl FreshnessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Missing => "missing",
            Self::ContentChanged => "content_changed",
            Self::Ambiguous => "ambiguous",
        }
    }
}

impl FromStr for FreshnessState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "current" => Ok(Self::Current),
            "missing" => Ok(Self::Missing),
            "content_changed" => Ok(Self::ContentChanged),
            "ambiguous" => Ok(Self::Ambiguous),
            _ => Err(format!("unknown triage freshness state: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Freshness {
    pub state: FreshnessState,
    pub reason: Option<String>,
    #[serde(default)]
    pub candidate_paths: Vec<String>,
}

impl Freshness {
    pub fn current() -> Self {
        Self {
            state: FreshnessState::Current,
            reason: None,
            candidate_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriageVerdict {
    Schedule,
    Delete,
    NeedsReplan,
    MergeInto,
    LeaveCaptured,
}

impl TriageVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Delete => "delete",
            Self::NeedsReplan => "needs_replan",
            Self::MergeInto => "merge_into",
            Self::LeaveCaptured => "leave_captured",
        }
    }
}

impl fmt::Display for TriageVerdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TriageVerdict {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "schedule" => Ok(Self::Schedule),
            "delete" => Ok(Self::Delete),
            "needs_replan" => Ok(Self::NeedsReplan),
            "merge_into" => Ok(Self::MergeInto),
            "leave_captured" => Ok(Self::LeaveCaptured),
            "" => Err("verdict must not be empty".to_string()),
            _ => Err(format!("unknown triage verdict: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct QueueCounts {
    pub total: i64,
    pub decided: i64,
    pub stale: i64,
    pub ambiguous: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageQueue {
    pub id: String,
    pub name: String,
    pub root: String,
    pub strip_suffix: String,
    pub source_spec: SourceSpec,
    pub status: QueueStatus,
    pub complete: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub counts: QueueCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageItem {
    pub queue_id: String,
    pub id: String,
    pub source_kind: SourceKind,
    pub path: Option<String>,
    pub source: Option<Value>,
    pub markdown: String,
    pub content_sha256: String,
    pub captured_at: DateTime<Utc>,
    pub freshness: Freshness,
    pub latest_revision: Option<TriageRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TriageRevision {
    pub event_id: i64,
    pub queue_id: String,
    pub item_id: String,
    pub revision: i64,
    pub verdict: Option<TriageVerdict>,
    pub target: Option<String>,
    pub content_sha256: String,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PatchField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> PatchField<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<T: Serialize> Serialize for PatchField<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Missing | Self::Null => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PatchField<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct DecisionPatch {
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub verdict: PatchField<TriageVerdict>,
    #[serde(default, skip_serializing_if = "PatchField::is_missing")]
    pub target: PatchField<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueDetail {
    pub queue: TriageQueue,
    pub catalog: Vec<TriageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RevisionPage {
    pub items: Vec<TriageRevision>,
    pub next_event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RefreshStats {
    pub current: usize,
    pub missing: usize,
    pub content_changed: usize,
    pub ambiguous: usize,
    pub added: usize,
    pub moved: usize,
    pub inline_replaced: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decision_patch_distinguishes_missing_null_and_value() {
        let missing: DecisionPatch = serde_json::from_value(json!({})).unwrap();
        let null: DecisionPatch = serde_json::from_value(json!({"verdict": null})).unwrap();
        let value: DecisionPatch = serde_json::from_value(json!({"verdict": "schedule"})).unwrap();

        assert_eq!(missing.verdict, PatchField::Missing);
        assert_eq!(null.verdict, PatchField::Null);
        assert_eq!(value.verdict, PatchField::Value(TriageVerdict::Schedule));
        assert!(serde_json::from_value::<DecisionPatch>(json!({"verdict": ""})).is_err());
    }

    #[test]
    fn sha256_matches_the_standard_test_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn output_models_keep_contractually_nullable_fields() {
        let freshness = serde_json::to_value(Freshness::current()).unwrap();
        assert_eq!(
            freshness,
            json!({"state":"current","reason":null,"candidate_paths":[]})
        );

        let revision = TriageRevision {
            event_id: 1,
            queue_id: "queue".to_string(),
            item_id: "item".to_string(),
            revision: 1,
            verdict: None,
            target: None,
            content_sha256: sha256_hex(b"item"),
            decided_at: Utc::now(),
        };
        let revision = serde_json::to_value(revision).unwrap();
        assert!(revision.get("verdict").unwrap().is_null());
        assert!(revision.get("target").unwrap().is_null());
    }
}
