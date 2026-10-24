//! Transient authority (ta) grants: envelope verification, the signer
//! registry, and the pure mapping from a verified grant to a notification
//! answer.
//!
//! Wire formats follow transient-authority DESIGN.md §4 "Wire formats
//! (normative)". Nothing in this module touches the database or axum, so all
//! of it is unit-testable offline; `grant_handlers` and `db` do the I/O.

use crate::models::{AnswerEnvelope, Notification, NotificationStatus, QuestionType};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, NaiveDateTime, Utc};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The only supported signature algorithm: ECDSA P-256 over SHA-256.
pub const ALG_ES256: &str = "es256";
/// The only supported record version (`record.v`).
pub const RECORD_VERSION: u64 = 1;
/// Env var overriding the signer registry path (tests, alternate setups).
pub const SIGNERS_ENV: &str = "SJBIS_SIGNERS";
/// Prefix of `AnswerEnvelope.via` for attested answers.
pub const VIA_PREFIX: &str = "ta:";

// ── Errors ───────────────────────────────────────────────────────────────

/// Every way a grant can be refused, with its HTTP status and a stable code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrantError {
    #[error("malformed grant: {0}")]
    Malformed(String),
    #[error("unsupported alg {0:?}; only \"es256\" is accepted")]
    UnsupportedAlg(String),
    #[error("record_sha256 does not match the SHA-256 of the canonical record")]
    HashMismatch,
    #[error("id {got:?} does not derive from the record (expected {expected:?})")]
    IdMismatch { expected: String, got: String },
    #[error("unknown signer {0:?}: not in the daemon's signer registry")]
    UnknownSigner(String),
    #[error("bad signature: {0}")]
    BadSignature(String),
    #[error("notification {0} not found")]
    NotificationNotFound(String),
    #[error("revoke target {0} not found")]
    TargetNotFound(String),
    #[error("grant {0} not found")]
    GrantNotFound(String),
    #[error("notification {0} is not an approval ask")]
    NotApproval(String),
    #[error("notification {0} is not open")]
    NotOpen(String),
    #[error("record.question is not byte-identical to the notification's question")]
    QuestionMismatch,
    #[error("grant id collision: {0} already exists; re-sign with a fresh issued_at")]
    IdCollision(String),
    #[error("grant {0} is not a once grant")]
    NotOnce(String),
    #[error("grant {0} already claimed")]
    AlreadyClaimed(String),
    #[error("signer registry unreadable: {0}")]
    Registry(String),
}

impl GrantError {
    /// HTTP status for this refusal (see README "Grants (ta)").
    pub fn http_status(&self) -> u16 {
        match self {
            GrantError::Malformed(_) => 400,
            GrantError::NotificationNotFound(_)
            | GrantError::TargetNotFound(_)
            | GrantError::GrantNotFound(_) => 404,
            GrantError::NotApproval(_)
            | GrantError::NotOpen(_)
            | GrantError::IdCollision(_)
            | GrantError::NotOnce(_)
            | GrantError::AlreadyClaimed(_) => 409,
            GrantError::UnsupportedAlg(_)
            | GrantError::HashMismatch
            | GrantError::IdMismatch { .. }
            | GrantError::UnknownSigner(_)
            | GrantError::BadSignature(_)
            | GrantError::QuestionMismatch => 422,
            GrantError::Registry(_) => 500,
        }
    }

    /// Stable machine-readable code (matches ta verdict names where they overlap).
    pub fn code(&self) -> &'static str {
        match self {
            GrantError::Malformed(_) => "malformed",
            GrantError::UnsupportedAlg(_) => "unsupported_alg",
            GrantError::HashMismatch => "hash_mismatch",
            GrantError::IdMismatch { .. } => "id_mismatch",
            GrantError::UnknownSigner(_) => "unknown_signer",
            GrantError::BadSignature(_) => "bad_signature",
            GrantError::NotificationNotFound(_) => "notification_not_found",
            GrantError::TargetNotFound(_) => "target_not_found",
            GrantError::GrantNotFound(_) => "not_found",
            GrantError::NotApproval(_) => "not_approval",
            GrantError::NotOpen(_) => "not_open",
            GrantError::QuestionMismatch => "question_mismatch",
            GrantError::IdCollision(_) => "id_collision",
            GrantError::NotOnce(_) => "not_once",
            GrantError::AlreadyClaimed(_) => "already_claimed",
            GrantError::Registry(_) => "registry_error",
        }
    }

    /// JSON error body: `{"error": "...", "code": "..."}`.
    pub fn body(&self) -> Value {
        serde_json::json!({ "error": self.to_string(), "code": self.code() })
    }
}

fn malformed(msg: impl Into<String>) -> GrantError {
    GrantError::Malformed(msg.into())
}

// ── Signer registry ─────────────────────────────────────────────────────

/// One `[[signer]]` entry of signers.toml.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Signer {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub device: Option<String>,
    /// Standard base64 of the 65-byte SEC1 uncompressed P-256 point.
    pub pubkey: String,
    /// When the key was added (quoted RFC 3339 string or a TOML datetime).
    #[serde(default)]
    pub added: Option<toml::Value>,
}

#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub signer: Signer,
    pub key: VerifyingKey,
}

/// The daemon's copy of the signer registry. It exists only to reject junk
/// early; verifiers trust their own pinned file, never this one.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    entries: Vec<RegistryEntry>,
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    signer: Vec<Signer>,
}

impl Registry {
    /// Parse signers.toml. Every entry must have a valid key and a unique id.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: RegistryFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut entries: Vec<RegistryEntry> = Vec::with_capacity(file.signer.len());
        for signer in file.signer {
            if signer.id.is_empty() {
                return Err("signer with empty id".to_string());
            }
            if entries.iter().any(|e| e.signer.id == signer.id) {
                return Err(format!("duplicate signer id {:?}", signer.id));
            }
            let key = decode_pubkey(&signer.pubkey)
                .map_err(|e| format!("signer {:?}: {e}", signer.id))?;
            entries.push(RegistryEntry { signer, key });
        }
        Ok(Self { entries })
    }

    /// Load the registry file. A missing file is an empty registry (every
    /// grant is then refused as unknown_signer); an unreadable or invalid
    /// file is an error.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn get(&self, id: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.signer.id == id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The pinned signers, in file order.
    pub fn signers(&self) -> impl Iterator<Item = &Signer> {
        self.entries.iter().map(|e| &e.signer)
    }
}

/// `$SJBIS_SIGNERS`, else `~/.config/sjbis/signers.toml`.
pub fn registry_path() -> PathBuf {
    if let Some(p) = std::env::var_os(SIGNERS_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/sjbis/signers.toml")
}

/// Decode a registry/enroll public key: standard base64 of exactly 65 bytes,
/// `0x04 ‖ x ‖ y`, on the P-256 curve.
pub fn decode_pubkey(b64: &str) -> Result<VerifyingKey, String> {
    let bytes = B64
        .decode(b64.as_bytes())
        .map_err(|e| format!("pubkey is not standard base64: {e}"))?;
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return Err(format!(
            "pubkey must be a 65-byte SEC1 uncompressed point (got {} bytes)",
            bytes.len()
        ));
    }
    VerifyingKey::from_sec1_bytes(&bytes)
        .map_err(|_| "pubkey is not a valid P-256 point".to_string())
}

// ── Envelope and record ─────────────────────────────────────────────────

/// The signed envelope as stored and served.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub id: String,
    pub record: Value,
    pub record_sha256: String,
    pub alg: String,
    pub sig: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approve,
    Deny,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Approve => "approve",
            Decision::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRecord {
    pub v: u64,
    pub ask_id: String,
    pub asked_by: String,
    pub question: String,
    pub decision: Decision,
    pub answer: Option<String>,
    pub note: Option<String>,
    pub issued_at: String,
    pub expires_at: String,
    pub once: Option<bool>,
    pub renews: Option<String>,
    pub signer: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeRecord {
    pub v: u64,
    pub target: String,
    pub reason: Option<String>,
    pub issued_at: String,
    pub signer: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollRecord {
    pub v: u64,
    pub signer_id: String,
    pub method: String,
    pub device: Option<String>,
    pub pubkey: String,
    pub issued_at: String,
    pub signer: String,
}

/// A typed record. Unknown fields, missing required fields and `null`
/// optionals are all malformed (matching ta-core).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Record {
    Grant(GrantRecord),
    Revoke(RevokeRecord),
    Enroll(EnrollRecord),
}

impl Record {
    pub fn kind(&self) -> &'static str {
        match self {
            Record::Grant(_) => "grant",
            Record::Revoke(_) => "revoke",
            Record::Enroll(_) => "enroll",
        }
    }

    pub fn signer(&self) -> &str {
        match self {
            Record::Grant(r) => &r.signer,
            Record::Revoke(r) => &r.signer,
            Record::Enroll(r) => &r.signer,
        }
    }

    fn version(&self) -> u64 {
        match self {
            Record::Grant(r) => r.v,
            Record::Revoke(r) => r.v,
            Record::Enroll(r) => r.v,
        }
    }

    fn issued_at_str(&self) -> &str {
        match self {
            Record::Grant(r) => &r.issued_at,
            Record::Revoke(r) => &r.issued_at,
            Record::Enroll(r) => &r.issued_at,
        }
    }

    /// The approval notification a grant answers (grants only).
    pub fn notification_id(&self) -> Option<&str> {
        match self {
            Record::Grant(r) => Some(&r.ask_id),
            _ => None,
        }
    }

    /// The earlier grant this one re-authorises (grants only).
    pub fn renews(&self) -> Option<&str> {
        match self {
            Record::Grant(r) => r.renews.as_deref(),
            _ => None,
        }
    }
}

/// Parse an RFC 3339 UTC timestamp with `Z` and second precision, exactly
/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, String> {
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
        .map_err(|_| format!("timestamp {s:?} is not RFC 3339 UTC YYYY-MM-DDTHH:MM:SSZ"))?;
    let dt = naive.and_utc();
    if format_timestamp(dt) != s {
        return Err(format!(
            "timestamp {s:?} is not in canonical second-precision form"
        ));
    }
    Ok(dt)
}

pub fn format_timestamp(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn reject_nulls(v: &Value, path: &str) -> Result<(), GrantError> {
    match v {
        Value::Null => Err(malformed(format!(
            "{path} is null; omit optional fields instead of sending null"
        ))),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(i, item)| reject_nulls(item, &format!("{path}[{i}]"))),
        Value::Object(map) => map
            .iter()
            .try_for_each(|(k, item)| reject_nulls(item, &format!("{path}.{k}"))),
        _ => Ok(()),
    }
}

/// A record that passed structural validation, with parsed times.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRecord {
    pub record: Record,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Structural validation of a record (no crypto): object, no nulls, known
/// kind, v = 1, required fields, canonical timestamps, valid enroll pubkey,
/// and for grants expires_at > issued_at.
pub fn parse_record(value: &Value) -> Result<ParsedRecord, GrantError> {
    if !value.is_object() {
        return Err(malformed("record must be a JSON object"));
    }
    reject_nulls(value, "record")?;
    let record: Record =
        serde_json::from_value(value.clone()).map_err(|e| malformed(format!("record: {e}")))?;
    if record.version() != RECORD_VERSION {
        return Err(malformed(format!(
            "record.v must be {RECORD_VERSION} (got {})",
            record.version()
        )));
    }
    if record.signer().is_empty() {
        return Err(malformed("record.signer is empty"));
    }
    let issued_at = parse_timestamp(record.issued_at_str())
        .map_err(|e| malformed(format!("record.issued_at: {e}")))?;
    let expires_at = match &record {
        Record::Grant(g) => {
            let exp = parse_timestamp(&g.expires_at)
                .map_err(|e| malformed(format!("record.expires_at: {e}")))?;
            if exp <= issued_at {
                return Err(malformed(
                    "record.expires_at must be after record.issued_at",
                ));
            }
            Some(exp)
        }
        Record::Enroll(e) => {
            decode_pubkey(&e.pubkey).map_err(|err| malformed(format!("record.pubkey: {err}")))?;
            None
        }
        Record::Revoke(_) => None,
    };
    Ok(ParsedRecord {
        record,
        issued_at,
        expires_at,
    })
}

/// RFC 8785 (JCS) canonical bytes of a record value.
pub fn canonical_bytes(record: &Value) -> Result<Vec<u8>, GrantError> {
    serde_jcs::to_vec(record).map_err(|e| malformed(format!("cannot canonicalize record: {e}")))
}

fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `record_sha256`: lowercase hex SHA-256 of the canonical bytes.
pub fn record_sha256(canonical: &[u8]) -> String {
    lower_hex(&Sha256::digest(canonical))
}

/// `id`: "ta-" + first 6 chars of lowercase unpadded RFC 4648 base32 of the
/// SHA-256 digest of the canonical bytes.
pub fn grant_id(canonical: &[u8]) -> String {
    let digest = Sha256::digest(canonical);
    let b32 = base32::encode(base32::Alphabet::Rfc4648Lower { padding: false }, &digest);
    format!("ta-{}", &b32[..6])
}

/// An envelope that verified against the registry.
#[derive(Debug, Clone)]
pub struct Verified {
    pub envelope: Envelope,
    pub parsed: ParsedRecord,
}

impl Verified {
    pub fn record(&self) -> &Record {
        &self.parsed.record
    }
}

/// Verify a raw envelope (DESIGN §4). Order of checks: envelope shape, alg,
/// record structure, record_sha256, id, signer, signature.
pub fn verify_envelope(raw: &Value, registry: &Registry) -> Result<Verified, GrantError> {
    let envelope: Envelope =
        serde_json::from_value(raw.clone()).map_err(|e| malformed(format!("envelope: {e}")))?;
    if envelope.alg != ALG_ES256 {
        return Err(GrantError::UnsupportedAlg(envelope.alg.clone()));
    }
    let parsed = parse_record(&envelope.record)?;
    let canonical = canonical_bytes(&envelope.record)?;
    if record_sha256(&canonical) != envelope.record_sha256 {
        return Err(GrantError::HashMismatch);
    }
    let expected_id = grant_id(&canonical);
    if expected_id != envelope.id {
        return Err(GrantError::IdMismatch {
            expected: expected_id,
            got: envelope.id.clone(),
        });
    }
    let signer_id = parsed.record.signer();
    let entry = registry
        .get(signer_id)
        .ok_or_else(|| GrantError::UnknownSigner(signer_id.to_string()))?;
    let der = B64
        .decode(envelope.sig.as_bytes())
        .map_err(|e| GrantError::BadSignature(format!("sig is not standard base64: {e}")))?;
    let sig = Signature::from_der(&der)
        .map_err(|_| GrantError::BadSignature("sig is not a DER ECDSA signature".to_string()))?;
    entry.key.verify(&canonical, &sig).map_err(|_| {
        GrantError::BadSignature(format!(
            "signature does not verify for signer {signer_id:?}"
        ))
    })?;
    Ok(Verified { envelope, parsed })
}

// ── Grant → notification answer ─────────────────────────────────────────

/// Check that a verified grant may answer `notif` (DESIGN §5 step 4).
pub fn check_grant_for_notification(
    grant: &GrantRecord,
    notif: &Notification,
) -> Result<(), GrantError> {
    if grant.ask_id != notif.id {
        return Err(GrantError::NotificationNotFound(grant.ask_id.clone()));
    }
    if notif.question_type != QuestionType::Approval {
        return Err(GrantError::NotApproval(notif.id.clone()));
    }
    if notif.status != NotificationStatus::Open {
        return Err(GrantError::NotOpen(notif.id.clone()));
    }
    if grant.question.as_bytes() != notif.question.as_bytes() {
        return Err(GrantError::QuestionMismatch);
    }
    Ok(())
}

/// The notification answer a grant records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantAnswer {
    /// `record.decision` ("approve" / "deny").
    pub answer: String,
    /// `record.answer` if present, else the decision.
    pub answer_label: String,
    /// `record.note`.
    pub note: Option<String>,
    /// "ta:" + signer id.
    pub via: String,
}

pub fn grant_answer(grant: &GrantRecord) -> GrantAnswer {
    GrantAnswer {
        answer: grant.decision.as_str().to_string(),
        answer_label: grant
            .answer
            .clone()
            .unwrap_or_else(|| grant.decision.as_str().to_string()),
        note: grant.note.clone(),
        via: format!("{VIA_PREFIX}{}", grant.signer),
    }
}

/// The envelope blocking waiters, SSE and reply_to receive when a grant
/// answers `notif` at `answered_at`.
pub fn grant_answer_envelope(
    notif: &Notification,
    grant: &GrantRecord,
    envelope: &Envelope,
    answered_at: DateTime<Utc>,
) -> AnswerEnvelope {
    let a = grant_answer(grant);
    AnswerEnvelope {
        id: notif.id.clone(),
        answer: Some(a.answer),
        answer_label: Some(a.answer_label),
        answered_at: Some(answered_at),
        latency_ms: Some(
            answered_at
                .signed_duration_since(notif.created_at)
                .num_milliseconds(),
        ),
        renderer: notif.question_type.to_string(),
        src: notif.src.clone(),
        via: a.via,
        note: a.note,
        grant: Some(serde_json::to_value(envelope).unwrap_or(Value::Null)),
        once: notif.once,
    }
}

// ── Stored grants and status ────────────────────────────────────────────

/// A row of the `grants` table.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredGrant {
    pub id: String,
    pub kind: String,
    pub notification_id: Option<String>,
    pub record: Value,
    pub record_sha256: String,
    pub sig: String,
    pub signer: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub renews: Option<String>,
    pub revoked_by: Option<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Daemon-side status of a grant. `expired` is computed at read time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantStatus {
    pub expired: bool,
    pub revoked_by: Option<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
}

/// GET /grants/{id} body.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GrantView {
    pub envelope: Envelope,
    pub status: GrantStatus,
}

/// A grant is expired once `now >= expires_at`. Records without an expiry
/// (revoke, enroll) never expire.
pub fn is_expired(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    expires_at.is_some_and(|exp| now >= exp)
}

impl StoredGrant {
    pub fn from_verified(v: &Verified, created_at: DateTime<Utc>) -> Self {
        let record = v.record();
        Self {
            id: v.envelope.id.clone(),
            kind: record.kind().to_string(),
            notification_id: record.notification_id().map(str::to_string),
            record: v.envelope.record.clone(),
            record_sha256: v.envelope.record_sha256.clone(),
            sig: v.envelope.sig.clone(),
            signer: record.signer().to_string(),
            issued_at: v.parsed.issued_at,
            expires_at: v.parsed.expires_at,
            renews: record.renews().map(str::to_string),
            revoked_by: None,
            claimed_by: None,
            claimed_at: None,
            created_at,
        }
    }

    pub fn envelope(&self) -> Envelope {
        Envelope {
            id: self.id.clone(),
            record: self.record.clone(),
            record_sha256: self.record_sha256.clone(),
            alg: ALG_ES256.to_string(),
            sig: self.sig.clone(),
        }
    }

    pub fn status_at(&self, now: DateTime<Utc>) -> GrantStatus {
        GrantStatus {
            expired: is_expired(self.expires_at, now),
            revoked_by: self.revoked_by.clone(),
            claimed_by: self.claimed_by.clone(),
            claimed_at: self.claimed_at,
        }
    }

    pub fn view_at(&self, now: DateTime<Utc>) -> GrantView {
        GrantView {
            envelope: self.envelope(),
            status: self.status_at(now),
        }
    }

    /// True for a kind-grant record with `once: true`.
    pub fn is_once(&self) -> bool {
        self.kind == "grant" && self.record.get("once") == Some(&Value::Bool(true))
    }

    /// Set `via` and `grant` on an answer envelope for the notification this
    /// grant answered (GET /wait on an already-answered approval ask).
    pub fn attach_to_answer(&self, envelope: &mut AnswerEnvelope) {
        envelope.via = format!("{VIA_PREFIX}{}", self.signer);
        envelope.grant = Some(serde_json::to_value(self.envelope()).unwrap_or(Value::Null));
    }
}

/// POST /grants/{id}/claim precondition: a once grant not yet claimed.
pub fn check_claim(grant: &StoredGrant) -> Result<(), GrantError> {
    if !grant.is_once() {
        return Err(GrantError::NotOnce(grant.id.clone()));
    }
    if grant.claimed_by.is_some() {
        return Err(GrantError::AlreadyClaimed(grant.id.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer as _};
    use serde_json::json;

    // ── helpers ──

    /// Deterministic throwaway key (no RNG needed).
    fn test_key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).expect("valid scalar")
    }

    fn pubkey_b64(key: &SigningKey) -> String {
        B64.encode(key.verifying_key().to_encoded_point(false).as_bytes())
    }

    fn registry_for(id: &str, key: &SigningKey) -> Registry {
        Registry::parse(&format!(
            "[[signer]]\nid = \"{id}\"\nmethod = \"software\"\ndevice = \"test\"\npubkey = \"{}\"\nadded = \"2026-10-23T00:00:00Z\"\n",
            pubkey_b64(key)
        ))
        .expect("registry parses")
    }

    fn sign(record: Value, key: &SigningKey) -> Value {
        let canonical = serde_jcs::to_vec(&record).unwrap();
        let sig: Signature = key.sign(&canonical);
        json!({
            "id": grant_id(&canonical),
            "record": record,
            "record_sha256": record_sha256(&canonical),
            "alg": "es256",
            "sig": B64.encode(sig.to_der().as_bytes()),
        })
    }

    fn grant_record() -> Value {
        json!({
            "v": 1,
            "kind": "grant",
            "ask_id": "sjbis-AbCdEfGh",
            "asked_by": "agent-x",
            "question": "Do you want to fire the missiles?",
            "decision": "approve",
            "answer": "plan B",
            "note": "Yes, but only the two on the left.",
            "issued_at": "2026-10-23T01:12:04Z",
            "expires_at": "2026-10-23T02:12:04Z",
            "once": false,
            "renews": "ta-3kp9ab",
            "signer": "test@unit"
        })
    }

    fn good() -> (Value, Registry) {
        let key = test_key();
        (sign(grant_record(), &key), registry_for("test@unit", &key))
    }

    fn notification(question: &str, qt: &str, status: &str) -> Notification {
        serde_json::from_value(json!({
            "id": "sjbis-AbCdEfGh",
            "agent_name": "agent-x",
            "sender": "agent-x",
            "src": "agent-x",
            "question": question,
            "question_type": qt,
            "urgency": 2,
            "blocking": true,
            "deadline": null,
            "reply_to": "stdout",
            "status": status,
            "created_at": "2026-10-23T01:12:00Z",
            "ttl_secs": 3600
        }))
        .unwrap()
    }

    // ── verification ──

    #[test]
    fn good_envelope_verifies() {
        let (env, reg) = good();
        let v = verify_envelope(&env, &reg).expect("verifies");
        assert_eq!(v.record().kind(), "grant");
        assert_eq!(v.record().signer(), "test@unit");
        assert_eq!(v.envelope.id, env["id"]);
        assert!(v.envelope.id.starts_with("ta-") && v.envelope.id.len() == 9);
        assert_eq!(
            v.parsed.expires_at,
            Some(parse_timestamp("2026-10-23T02:12:04Z").unwrap())
        );
    }

    #[test]
    fn key_order_in_the_posted_record_does_not_matter() {
        let (env, reg) = good();
        // Re-serialize the record with keys in reverse order: JCS must
        // recompute the same canonical bytes.
        let mut text = String::from("{");
        let rec = env["record"].as_object().unwrap();
        let mut keys: Vec<_> = rec.keys().collect();
        keys.reverse();
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                text.push_str(" , ");
            }
            text.push_str(&format!("{:?} : {}", k, rec[*k]));
        }
        text.push('}');
        let mut env2 = env.clone();
        env2["record"] = serde_json::from_str(&text).unwrap();
        verify_envelope(&env2, &reg).expect("verifies regardless of key order");
    }

    #[test]
    fn tampered_question_is_hash_mismatch() {
        let (mut env, reg) = good();
        env["record"]["question"] = json!("Do you want to fire ALL the missiles?");
        assert_eq!(
            verify_envelope(&env, &reg).unwrap_err(),
            GrantError::HashMismatch
        );
    }

    #[test]
    fn tampered_question_with_recomputed_hash_and_id_fails_signature() {
        let (mut env, reg) = good();
        env["record"]["question"] = json!("Do you want to fire ALL the missiles?");
        let canonical = canonical_bytes(&env["record"]).unwrap();
        env["record_sha256"] = json!(record_sha256(&canonical));
        env["id"] = json!(grant_id(&canonical));
        let err = verify_envelope(&env, &reg).unwrap_err();
        assert!(matches!(err, GrantError::BadSignature(_)), "{err:?}");
        assert_eq!(err.http_status(), 422);
    }

    #[test]
    fn tampered_sig_fails() {
        let (mut env, reg) = good();
        let mut der = B64.decode(env["sig"].as_str().unwrap()).unwrap();
        let last = der.len() - 1;
        der[last] ^= 0x01;
        env["sig"] = json!(B64.encode(&der));
        let err = verify_envelope(&env, &reg).unwrap_err();
        assert!(matches!(err, GrantError::BadSignature(_)), "{err:?}");
    }

    #[test]
    fn signature_from_another_key_fails() {
        let (env, _) = good();
        let other = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let reg = registry_for("test@unit", &other);
        assert!(matches!(
            verify_envelope(&env, &reg).unwrap_err(),
            GrantError::BadSignature(_)
        ));
    }

    #[test]
    fn wrong_id_fails() {
        let (mut env, reg) = good();
        env["id"] = json!("ta-aaaaaa");
        let err = verify_envelope(&env, &reg).unwrap_err();
        assert!(matches!(err, GrantError::IdMismatch { .. }), "{err:?}");
        assert_eq!(err.http_status(), 422);
        // Uppercase base32 is not the canonical id either.
        let (mut env, reg) = good();
        env["id"] = json!(
            env["id"]
                .as_str()
                .unwrap()
                .to_uppercase()
                .replacen("TA-", "ta-", 1)
        );
        assert!(matches!(
            verify_envelope(&env, &reg).unwrap_err(),
            GrantError::IdMismatch { .. }
        ));
    }

    #[test]
    fn unknown_signer_fails() {
        let key = test_key();
        let mut rec = grant_record();
        rec["signer"] = json!("ghost@nowhere");
        let env = sign(rec, &key);
        let reg = registry_for("test@unit", &key);
        let err = verify_envelope(&env, &reg).unwrap_err();
        assert_eq!(err, GrantError::UnknownSigner("ghost@nowhere".into()));
        assert_eq!(err.http_status(), 422);
        assert_eq!(err.code(), "unknown_signer");
        // Empty registry: everything is unknown.
        assert!(matches!(
            verify_envelope(&env, &Registry::default()).unwrap_err(),
            GrantError::UnknownSigner(_)
        ));
    }

    #[test]
    fn bad_base64_sig_fails() {
        for bad in [
            "not base64!!",
            "MEQCIBal",
            "",
            "MEQCIBaldMHJHdRHslf0VQJPcAOKsItQsQuwgJKgYzmTRkoi",
        ] {
            let (mut env, reg) = good();
            env["sig"] = json!(bad);
            let err = verify_envelope(&env, &reg).unwrap_err();
            assert!(
                matches!(err, GrantError::BadSignature(_)),
                "{bad:?}: {err:?}"
            );
        }
        // URL-safe alphabet is not standard base64.
        let (mut env, reg) = good();
        let url_safe = env["sig"]
            .as_str()
            .unwrap()
            .replace('+', "-")
            .replace('/', "_");
        if url_safe != env["sig"].as_str().unwrap() {
            env["sig"] = json!(url_safe);
            assert!(matches!(
                verify_envelope(&env, &reg).unwrap_err(),
                GrantError::BadSignature(_)
            ));
        }
    }

    #[test]
    fn non_canonical_record_sha256_fails() {
        let (env, reg) = good();
        let hex = env["record_sha256"].as_str().unwrap().to_string();
        for bad in [
            hex.to_uppercase(),
            format!("{hex} "),
            hex[..63].to_string(),
            String::new(),
        ] {
            let mut e = env.clone();
            e["record_sha256"] = json!(bad);
            assert_eq!(
                verify_envelope(&e, &reg).unwrap_err(),
                GrantError::HashMismatch
            );
        }
        // Hash of the non-canonical (pretty-printed) serialization is wrong.
        let mut e = env.clone();
        let pretty = serde_json::to_vec_pretty(&env["record"]).unwrap();
        e["record_sha256"] = json!(record_sha256(&pretty));
        assert_eq!(
            verify_envelope(&e, &reg).unwrap_err(),
            GrantError::HashMismatch
        );
    }

    #[test]
    fn wrong_alg_fails() {
        let (mut env, reg) = good();
        env["alg"] = json!("ES256");
        let err = verify_envelope(&env, &reg).unwrap_err();
        assert_eq!(err, GrantError::UnsupportedAlg("ES256".into()));
    }

    #[test]
    fn malformed_envelopes_are_400() {
        let key = test_key();
        let reg = registry_for("test@unit", &key);
        let cases: Vec<Value> = vec![
            json!("just a string"),
            json!({"id": "ta-x"}),
            {
                let mut e = sign(grant_record(), &key);
                e["extra"] = json!(1);
                e
            },
            sign(json!([1, 2]), &key),
            {
                let mut r = grant_record();
                r["note"] = Value::Null;
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["v"] = json!(2);
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["kind"] = json!("bless");
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["decision"] = json!("maybe");
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r.as_object_mut().unwrap().remove("question");
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["issued_at"] = json!("2026-10-23T01:12:04.123Z");
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["expires_at"] = json!("2026-10-23T02:12:04+00:00");
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["expires_at"] = r["issued_at"].clone();
                sign(r, &key)
            },
            {
                let mut r = grant_record();
                r["once"] = json!("yes");
                sign(r, &key)
            },
            sign(
                json!({"v": 1, "kind": "enroll", "signer_id": "yk@pi", "method": "yubikey",
                       "pubkey": "BAAA", "issued_at": "2026-10-23T00:00:00Z", "signer": "test@unit"}),
                &key,
            ),
        ];
        for (i, env) in cases.iter().enumerate() {
            let err = verify_envelope(env, &reg).unwrap_err();
            assert!(matches!(err, GrantError::Malformed(_)), "case {i}: {err:?}");
            assert_eq!(err.http_status(), 400, "case {i}");
        }
    }

    #[test]
    fn revoke_and_enroll_records_verify() {
        let key = test_key();
        let reg = registry_for("test@unit", &key);
        let revoke = sign(
            json!({"v": 1, "kind": "revoke", "target": "ta-7f3k2q", "reason": "changed my mind",
                   "issued_at": "2026-10-23T01:22:30Z", "signer": "test@unit"}),
            &key,
        );
        let v = verify_envelope(&revoke, &reg).unwrap();
        assert!(matches!(v.record(), Record::Revoke(r) if r.target == "ta-7f3k2q"));
        assert_eq!(v.parsed.expires_at, None);

        let other = SigningKey::from_slice(&[3u8; 32]).unwrap();
        let enroll = sign(
            json!({"v": 1, "kind": "enroll", "signer_id": "yubikey@pi-console", "method": "yubikey",
                   "device": "Pi 400 console", "pubkey": pubkey_b64(&other),
                   "issued_at": "2026-10-23T00:00:00Z", "signer": "test@unit"}),
            &key,
        );
        let v = verify_envelope(&enroll, &reg).unwrap();
        assert_eq!(v.record().kind(), "enroll");
        let stored = StoredGrant::from_verified(&v, Utc::now());
        assert_eq!(stored.notification_id, None);
        assert_eq!(stored.expires_at, None);
    }

    #[test]
    fn id_and_hash_derivation_known_answer() {
        // SHA-256("") = e3b0c442...; base32 lower of that digest starts "4oymiq".
        assert_eq!(
            record_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(grant_id(b""), "ta-4oymiq");
    }

    #[test]
    fn timestamps_are_strict() {
        assert!(parse_timestamp("2026-10-23T01:12:04Z").is_ok());
        for bad in [
            "2026-10-23T01:12:04",
            "2026-10-23T01:12:04.000Z",
            "2026-10-23 01:12:04Z",
            "2026-10-23T01:12:04+00:00",
            "2026-10-23t01:12:04z",
            "2026-1-23T01:12:04Z",
        ] {
            assert!(parse_timestamp(bad).is_err(), "{bad}");
        }
    }

    // ── registry ──

    #[test]
    fn registry_parses_design_shape() {
        let key = test_key();
        let toml_text = format!(
            "[[signer]]\nid     = \"faceid@iphone\"\nmethod = \"faceid\"\ndevice = \"Stephen's iPhone\"\npubkey = \"{}\"   # SEC1\nadded  = \"2026-10-23T00:00:00Z\"\n\n[[signer]]\nid     = \"yubikey@pi-console\"\nmethod = \"yubikey\"\npubkey = \"{}\"\nadded = 2026-10-23T00:00:00Z\n",
            pubkey_b64(&key),
            pubkey_b64(&SigningKey::from_slice(&[5u8; 32]).unwrap())
        );
        let reg = Registry::parse(&toml_text).unwrap();
        assert_eq!(reg.len(), 2);
        let e = reg.get("faceid@iphone").unwrap();
        assert_eq!(e.signer.method, "faceid");
        assert_eq!(e.signer.device.as_deref(), Some("Stephen's iPhone"));
        assert_eq!(&e.key, key.verifying_key());
        assert_eq!(reg.get("yubikey@pi-console").unwrap().signer.device, None);
        assert!(reg.get("nobody").is_none());
        assert!(Registry::parse("").unwrap().is_empty());
    }

    #[test]
    fn registry_rejects_bad_entries() {
        let key = pubkey_b64(&test_key());
        let compressed = B64.encode(test_key().verifying_key().to_encoded_point(true).as_bytes());
        let bad = [
            format!("[[signer]]\nid = \"a\"\nmethod = \"m\"\npubkey = \"{compressed}\"\n"),
            "[[signer]]\nid = \"a\"\nmethod = \"m\"\npubkey = \"not base64\"\n".to_string(),
            format!(
                "[[signer]]\nid = \"a\"\nmethod = \"m\"\npubkey = \"{}\"\n",
                B64.encode([4u8; 65])
            ),
            format!("[[signer]]\nmethod = \"m\"\npubkey = \"{key}\"\n"),
            format!(
                "[[signer]]\nid = \"a\"\nmethod = \"m\"\npubkey = \"{key}\"\n[[signer]]\nid = \"a\"\nmethod = \"m\"\npubkey = \"{key}\"\n"
            ),
            "signer = 3".to_string(),
        ];
        for text in bad {
            assert!(Registry::parse(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn registry_load_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::load(&dir.path().join("signers.toml")).unwrap();
        assert!(reg.is_empty());
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "[[signer]]\nid = 1\n").unwrap();
        assert!(Registry::load(&path).is_err());
    }

    // ── cross-check against the ta-core fixture (independent implementation) ──

    const FIXTURE: &str = include_str!("../tests/fixtures/ta/grant-v1.json");
    const FIXTURE_SIGNERS: &str = include_str!("../tests/fixtures/ta/signers.toml");

    #[test]
    fn ta_fixture_cases_verify_and_reproduce_id_and_hash() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(fixture["format"], "ta grant fixture v1");
        let reg = Registry::parse(FIXTURE_SIGNERS).unwrap();
        assert_eq!(reg.len(), 1);
        // The embedded registry copy agrees with signers.toml.
        assert_eq!(
            fixture["registry"]["signers_toml"].as_str().unwrap(),
            FIXTURE_SIGNERS
        );
        let verify_at = parse_timestamp(fixture["verify_at"].as_str().unwrap()).unwrap();
        assert_eq!(format_timestamp(verify_at), "2026-10-23T01:25:00Z");
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 3);
        let mut names = Vec::new();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let env = &case["envelope"];
            let canonical = canonical_bytes(&env["record"]).unwrap();
            assert_eq!(
                std::str::from_utf8(&canonical).unwrap(),
                case["canonical"].as_str().unwrap(),
                "{name}: canonical bytes"
            );
            assert_eq!(
                record_sha256(&canonical),
                case["record_sha256"],
                "{name}: record_sha256"
            );
            assert_eq!(grant_id(&canonical), case["id"], "{name}: id");
            let v = verify_envelope(env, &reg).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(v.envelope.id, case["id"].as_str().unwrap(), "{name}");
            assert_eq!(
                serde_json::to_value(&v.envelope).unwrap(),
                *env,
                "{name}: round-trip"
            );
            // Expiry evaluated at the fixture's fixed verify_at.
            let expect = &case["expect_at_verify_at"];
            let status = StoredGrant::from_verified(&v, verify_at).status_at(verify_at);
            assert!(!status.expired, "{name}: not expired at verify_at");
            match (expect["verdict"].as_str().unwrap(), v.record()) {
                ("valid", Record::Grant(g)) => assert_eq!(g.decision, Decision::Approve, "{name}"),
                ("denied", Record::Grant(g)) => assert_eq!(g.decision, Decision::Deny, "{name}"),
                ("valid", _) => {}
                (verdict, rec) => panic!("{name}: unexpected verdict {verdict} for {}", rec.kind()),
            }
            if let (Some(q), Record::Grant(g)) = (expect["question"].as_str(), v.record()) {
                assert_eq!(g.question.as_bytes(), q.as_bytes(), "{name}: question");
            }
            names.push(name.to_string());
        }
        for want in ["approve", "deny", "revoke"] {
            assert!(
                names.iter().any(|n| n == want),
                "fixture case {want} missing"
            );
        }
        // One second past the deny grant's expiry it reads as expired.
        let deny = cases.iter().find(|c| c["name"] == "deny").unwrap();
        let v = verify_envelope(&deny["envelope"], &reg).unwrap();
        let stored = StoredGrant::from_verified(&v, verify_at);
        assert!(
            stored
                .status_at(parse_timestamp("2026-10-23T01:30:00Z").unwrap())
                .expired
        );
    }

    #[test]
    fn ta_fixture_high_s_twin_verifies() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let reg = Registry::parse(FIXTURE_SIGNERS).unwrap();
        let variants = fixture["signature_variants"].as_array().unwrap();
        assert!(!variants.is_empty());
        for case in variants {
            let name = case["name"].as_str().unwrap();
            let v =
                verify_envelope(&case["envelope"], &reg).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(v.envelope.id, "ta-ucfuud", "{name}: same id as approve");
        }
    }

    #[test]
    fn ta_fixture_record_with_unknown_field_is_malformed() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let reg = Registry::parse(FIXTURE_SIGNERS).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let mut env = case["envelope"].clone();
            env["record"]["extra"] = json!("x");
            let canonical = canonical_bytes(&env["record"]).unwrap();
            env["record_sha256"] = json!(record_sha256(&canonical));
            env["id"] = json!(grant_id(&canonical));
            let err = verify_envelope(&env, &reg).unwrap_err();
            assert_eq!(err.code(), "malformed", "{}: {err}", case["name"]);
        }
    }

    #[test]
    fn ta_fixture_negative_cases_fail_with_matching_verdict() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let reg = Registry::parse(FIXTURE_SIGNERS).unwrap();
        let negatives = fixture["negative"].as_array().unwrap();
        assert!(!negatives.is_empty());
        for case in negatives {
            let name = case["name"].as_str().unwrap();
            let err = verify_envelope(&case["envelope"], &reg)
                .err()
                .unwrap_or_else(|| panic!("{name}: should not verify"));
            assert_eq!(
                err.code(),
                case["expect"]["verdict"].as_str().unwrap(),
                "{name}: {err}"
            );
        }
    }

    #[test]
    fn ta_fixture_grants_answer_their_asks() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let reg = Registry::parse(FIXTURE_SIGNERS).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let v = verify_envelope(&case["envelope"], &reg).unwrap();
            let Record::Grant(g) = v.record() else {
                continue;
            };
            let mut n = notification(&g.question, "approval", "open");
            n.id = g.ask_id.clone();
            assert_eq!(check_grant_for_notification(g, &n), Ok(()));
            let a = grant_answer(g);
            assert_eq!(a.answer, g.decision.as_str());
            assert_eq!(a.via, "ta:dev@fixtures");
        }
    }

    // ── grant → notification ──

    fn grant_of(v: &Value) -> GrantRecord {
        match parse_record(v).unwrap().record {
            Record::Grant(g) => g,
            other => panic!("not a grant: {other:?}"),
        }
    }

    #[test]
    fn grant_answer_mapping() {
        let g = grant_of(&grant_record());
        assert_eq!(
            grant_answer(&g),
            GrantAnswer {
                answer: "approve".into(),
                answer_label: "plan B".into(),
                note: Some("Yes, but only the two on the left.".into()),
                via: "ta:test@unit".into(),
            }
        );
        let mut r = grant_record();
        let obj = r.as_object_mut().unwrap();
        obj.remove("answer");
        obj.remove("note");
        obj.insert("decision".into(), json!("deny"));
        let a = grant_answer(&grant_of(&r));
        assert_eq!(a.answer, "deny");
        assert_eq!(a.answer_label, "deny");
        assert_eq!(a.note, None);
    }

    #[test]
    fn grant_answer_envelope_carries_full_envelope() {
        let (env, reg) = good();
        let v = verify_envelope(&env, &reg).unwrap();
        let Record::Grant(g) = v.record() else {
            panic!()
        };
        let notif = notification("Do you want to fire the missiles?", "approval", "open");
        let at = parse_timestamp("2026-10-23T01:12:05Z").unwrap();
        let ae = grant_answer_envelope(&notif, g, &v.envelope, at);
        assert_eq!(ae.id, "sjbis-AbCdEfGh");
        assert_eq!(ae.answer.as_deref(), Some("approve"));
        assert_eq!(ae.answer_label.as_deref(), Some("plan B"));
        assert_eq!(ae.via, "ta:test@unit");
        assert_eq!(ae.renderer, "approval");
        assert_eq!(ae.latency_ms, Some(5000));
        assert_eq!(ae.answered_at, Some(at));
        assert_eq!(ae.grant.as_ref().unwrap(), &env);
        // Serialized for /wait it round-trips as an Envelope.
        let wire = serde_json::to_value(&ae).unwrap();
        let back: Envelope = serde_json::from_value(wire["grant"].clone()).unwrap();
        assert_eq!(back, v.envelope);
    }

    #[test]
    fn grant_checks_against_notification() {
        let g = grant_of(&grant_record());
        let q = "Do you want to fire the missiles?";
        assert_eq!(
            check_grant_for_notification(&g, &notification(q, "approval", "open")),
            Ok(())
        );
        assert_eq!(
            check_grant_for_notification(&g, &notification(q, "yesno", "open")),
            Err(GrantError::NotApproval("sjbis-AbCdEfGh".into()))
        );
        for status in ["answered", "timed_out", "dismissed", "cancelled"] {
            let err =
                check_grant_for_notification(&g, &notification(q, "approval", status)).unwrap_err();
            assert_eq!(err, GrantError::NotOpen("sjbis-AbCdEfGh".into()));
            assert_eq!(err.http_status(), 409);
        }
        for other in [
            "Do you want to fire the missiles? ",
            "do you want to fire the missiles?",
            "Do you want to fire the missiles\u{FF1F}",
        ] {
            let err = check_grant_for_notification(&g, &notification(other, "approval", "open"))
                .unwrap_err();
            assert_eq!(err, GrantError::QuestionMismatch);
            assert_eq!(err.http_status(), 422);
        }
        let mut n = notification(q, "approval", "open");
        n.id = "sjbis-other".into();
        assert_eq!(
            check_grant_for_notification(&g, &n)
                .unwrap_err()
                .http_status(),
            404
        );
    }

    #[test]
    fn status_expiry_and_claims() {
        let (env, reg) = good();
        let v = verify_envelope(&env, &reg).unwrap();
        let mut stored = StoredGrant::from_verified(&v, Utc::now());
        assert_eq!(stored.kind, "grant");
        assert_eq!(stored.notification_id.as_deref(), Some("sjbis-AbCdEfGh"));
        assert_eq!(stored.renews.as_deref(), Some("ta-3kp9ab"));
        assert_eq!(stored.envelope(), v.envelope);

        let before = parse_timestamp("2026-10-23T02:12:03Z").unwrap();
        let at = parse_timestamp("2026-10-23T02:12:04Z").unwrap();
        assert!(!stored.status_at(before).expired);
        assert!(stored.status_at(at).expired);
        let view = serde_json::to_value(stored.view_at(before)).unwrap();
        assert_eq!(
            view["status"],
            json!({"expired": false, "revoked_by": null, "claimed_by": null, "claimed_at": null})
        );
        assert_eq!(view["envelope"], env);

        // once: false → cannot be claimed.
        assert_eq!(
            check_claim(&stored),
            Err(GrantError::NotOnce(stored.id.clone()))
        );
        stored.record["once"] = json!(true);
        assert_eq!(check_claim(&stored), Ok(()));
        stored.claimed_by = Some("agent-y".into());
        let err = check_claim(&stored).unwrap_err();
        assert_eq!(err, GrantError::AlreadyClaimed(stored.id.clone()));
        assert_eq!(err.http_status(), 409);
    }

    #[test]
    fn attach_to_answer_sets_via_and_grant() {
        let (env, reg) = good();
        let stored = StoredGrant::from_verified(&verify_envelope(&env, &reg).unwrap(), Utc::now());
        let mut ae = AnswerEnvelope {
            id: "sjbis-AbCdEfGh".into(),
            answer: Some("approve".into()),
            answer_label: Some("plan B".into()),
            answered_at: None,
            latency_ms: None,
            renderer: "approval".into(),
            src: "agent-x".into(),
            via: "dashboard".into(),
            note: None,
            grant: None,
            once: None,
        };
        stored.attach_to_answer(&mut ae);
        assert_eq!(ae.via, "ta:test@unit");
        assert_eq!(ae.grant, Some(env));
    }

    #[test]
    fn error_codes_and_collision_message() {
        let err = GrantError::IdCollision("ta-abcdef".into());
        assert_eq!(err.http_status(), 409);
        // ta's client recognises a collision by this word in the error text.
        assert!(err.to_string().contains("collision"));
        assert_eq!(err.body()["code"], "id_collision");
        assert_eq!(GrantError::Registry("x".into()).http_status(), 500);
        assert_eq!(GrantError::TargetNotFound("t".into()).http_status(), 404);
    }
}
