use crate::entities::EntityGroups;
use crate::models::*;
use chrono::Utc;

/// Evaluate rules against a notification. Returns the modified notification and
/// optionally an auto-answer value.
///
/// Rules are evaluated in priority order (highest first). Higher priority rules
/// can override lower priority ones. For example:
/// - Priority 10: "mute all iMessage" (low priority, applied first)
/// - Priority 20: "surface iMessage from Jeff" (high priority, overrides mute)
pub fn evaluate(rules: &[Rule], mut notif: Notification) -> (Notification, Option<String>) {
    let now = Utc::now();
    let mut auto_answer: Option<String> = None;

    // Sort by priority descending — highest priority wins (applied last, can override)
    let mut sorted: Vec<&Rule> = rules.iter().collect();
    sorted.sort_by_key(|r| -r.priority);

    for rule in sorted {
        if !rule.active {
            continue;
        }
        // Check expiration
        if let Some(expires) = rule.expires_at {
            if now > expires {
                continue;
            }
        }

        // Parse compiled JSON if present
        let Some(compiled) = &rule.compiled else {
            // No compiled JSON — apply raw rule semantics
            if rule.mute {
                notif.status = NotificationStatus::Muted;
            }
            continue;
        };

        // Check match conditions
        let matched = match_rule(compiled, &notif);
        if !matched {
            continue;
        }

        // Apply action
        if let Some(action) = compiled.get("action").and_then(|v| v.as_str()) {
            match action {
                "mute" => {
                    notif.status = NotificationStatus::Muted;
                }
                "surface" => {
                    notif.status = NotificationStatus::Open;
                }
                a if a.starts_with("reprioritize:") => {
                    if let Some(val) = a.strip_prefix("reprioritize:").and_then(|s| s.parse::<i32>().ok()) {
                        notif.urgency = val.clamp(0, 5);
                    }
                }
                a if a.starts_with("snooze:") => {
                    if let Some(dur_str) = a.strip_prefix("snooze:") {
                        if let Ok(d) = crate::models::parse_deadline(dur_str) {
                            notif.deadline = Some(d);
                        }
                    }
                }
                a if a.starts_with("auto_answer:") => {
                    auto_answer = a.strip_prefix("auto_answer:").map(|s| s.to_string());
                }
                _ => {
                    // pass through
                }
            }
        }
    }

    (notif, auto_answer)
}

fn match_rule(compiled: &serde_json::Value, notif: &Notification) -> bool {
    let Some(match_obj) = compiled.get("match") else {
        return true; // no match criteria = matches everything
    };

    // Check agent match
    if let Some(agent_val) = match_obj.get("agent") {
        let agent_match = match agent_val {
            serde_json::Value::String(s) => s == "*" || s == &notif.agent_name,
            serde_json::Value::Array(arr) => arr.iter().any(|v| {
                v.as_str().map_or(false, |s| s == "*" || s == &notif.agent_name)
            }),
            _ => false,
        };
        if !agent_match {
            return false;
        }
    }

    // Check urgency_min
    if let Some(min) = match_obj.get("urgency_min").and_then(|v| v.as_i64()) {
        if (notif.urgency as i64) < min {
            return false;
        }
    }

    // Check urgency_max
    if let Some(max) = match_obj.get("urgency_max").and_then(|v| v.as_i64()) {
        if (notif.urgency as i64) > max {
            return false;
        }
    }

    // Check question_includes (partial match, case-insensitive)
    if let Some(substr) = match_obj.get("question_includes").and_then(|v| v.as_str()) {
        if !notif.question.to_lowercase().contains(&substr.to_lowercase()) {
            return false;
        }
    }

    // Check source_contains (partial match, case-insensitive)
    if let Some(source_val) = match_obj.get("source_contains") {
        let source_match = match source_val {
            serde_json::Value::String(s) => {
                s == "*" || notif.src.to_lowercase().contains(&s.to_lowercase())
            }
            serde_json::Value::Array(arr) => arr.iter().any(|v| {
                v.as_str().map_or(false, |s| {
                    s == "*" || notif.src.to_lowercase().contains(&s.to_lowercase())
                })
            }),
            _ => false,
        };
        if !source_match {
            return false;
        }
    }

    // Check source_is (exact match, case-insensitive)
    if let Some(source_val) = match_obj.get("source_is") {
        let source_match = match source_val {
            serde_json::Value::String(s) => {
                s == "*" || notif.src.to_lowercase() == s.to_lowercase()
            }
            serde_json::Value::Array(arr) => arr.iter().any(|v| {
                v.as_str().map_or(false, |s| {
                    s == "*" || notif.src.to_lowercase() == s.to_lowercase()
                })
            }),
            _ => false,
        };
        if !source_match {
            return false;
        }
    }

    true
}

/// Compile natural language rule text into structured JSON.
///
/// This is a deterministic, offline compiler — no AI, no network.
/// It handles common phrasings so users don't need to remember syntax.
///
/// Supported patterns:
/// - "mute [agent]" / "mute all [agent]" / "silence [agent]"
/// - "mute all" / "mute everyone" / "mute everything" → wildcard agent "*"
/// - "surface [agent] from [contact]" / "allow [agent] from [contact]"
/// - "allow [agent] from [group]" → entity group expansion
/// - "only [agent] from [contact]" / "just [agent] from [contact]"
/// - "mute [agent] except [contact1, contact2]" → creates mute + surface rules
/// - "mute everyone but [group]" → wildcard mute + surface exceptions
/// - "... for 1h" / "... until 5pm" → duration parsing
/// - "dismiss [agent]" / "auto-ack [agent]" → auto_answer
/// - "urgent only" / "only urgent" → urgency_min filter
///
/// Returns `None` if the phrasing is unrecognized — caller should try AI fallback.
pub fn compile(text: &str, entities: &EntityGroups) -> Option<(serde_json::Value, Option<String>)> {
    // ── Extract duration suffix like "... for 1h" or "... for 30 minutes" ──
    let (text_without_duration, duration_str) = extract_duration(text);
    let lower_no_dur = text_without_duration.to_lowercase();

    // ── Pattern: "mute all" / "mute everyone" / "mute everything" / "silence all" ──
    if is_mute_all(&lower_no_dur) {
        let compiled = serde_json::json!({
            "action": "mute",
            "match": { "agent": "*" }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "mute [agent]" / "silence [agent]" / "mute all [agent]" ──
    if let Some(agent) = parse_mute_agent(&lower_no_dur, &text_without_duration) {
        let compiled = serde_json::json!({
            "action": "mute",
            "match": { "agent": agent }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "mute [agent] except/but [contacts]" or "mute all except [contacts]" ──
    if let Some((agent, contacts)) = parse_mute_except(&lower_no_dur, &text_without_duration, entities) {
        let expanded: Vec<String> = contacts.iter()
            .flat_map(|c| entities.expand(c))
            .collect();
        let _compiled = serde_json::json!({
            "action": "mute",
            "match": { "agent": agent, "not_source_contains": expanded }
        });
        // Negation is handled by the caller (create_rule) which detects "mute ... except/but"
        // and generates mute-all + surface-exceptions rules. We return None here so the
        // handler's special-case logic kicks in.
        return None; // Complex negation — let AI or multi-rule generator handle it
    }

    // ── Pattern: "only allow [agent] from [contacts]" / "only [agent] from [contacts]" ──
    if let Some((agent, contacts)) = parse_only_allow(&lower_no_dur, &text_without_duration) {
        let expanded: Vec<String> = contacts.iter()
            .flat_map(|c| entities.expand(c))
            .collect();
        let compiled = serde_json::json!({
            "action": "surface",
            "match": {
                "agent": agent,
                "source_contains": expanded
            }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "surface [agent] from [contact]" / "allow [agent] from [contact]" / "unmute [agent] from [contact]" ──
    if let Some((agent, contacts)) = parse_surface_from(&lower_no_dur, &text_without_duration) {
        let expanded: Vec<String> = contacts.iter()
            .flat_map(|c| entities.expand(c))
            .collect();
        let compiled = serde_json::json!({
            "action": "surface",
            "match": {
                "agent": agent,
                "source_contains": expanded
            }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "surface [agent]" / "allow [agent]" / "unmute [agent]" ──
    if let Some(agent) = parse_surface_agent(&lower_no_dur, &text_without_duration) {
        let compiled = serde_json::json!({
            "action": "surface",
            "match": { "agent": agent }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "urgent only" / "only urgent" / "just urgent" → urgency_min: 4 ──
    if is_urgent_only(&lower_no_dur) {
        let compiled = serde_json::json!({
            "action": "surface",
            "match": { "urgency_min": 4 }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "auto-ack [agent]" / "dismiss [agent]" / "auto-answer [agent]" ──
    if let Some(agent) = parse_auto_ack(&lower_no_dur, &text_without_duration) {
        let compiled = serde_json::json!({
            "action": "auto_answer:(acknowledged)",
            "match": { "agent": agent }
        });
        return Some((compiled, duration_str));
    }

    // ── Pattern: "reprioritize [agent] to [N]" / "set [agent] urgency to [N]" ──
    if let Some((agent, urgency)) = parse_reprioritize(&lower_no_dur, &text_without_duration) {
        let compiled = serde_json::json!({
            "action": format!("reprioritize:{}", urgency),
            "match": { "agent": agent }
        });
        return Some((compiled, duration_str));
    }

    None
}

/// Parse a "mute ... except ..." or "mute ... but ..." pattern.
/// Automatically strips duration suffixes ("for 1h", "until 5pm") from contacts.
/// Returns (agent, contacts) if matched.
pub fn parse_mute_except_text(text: &str, entities: &EntityGroups) -> Option<(String, Vec<String>)> {
    let lower = text.to_lowercase();
    let prefixes = ["mute ", "silence ", "quiet ", "suppress "];
    for prefix in &prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let rest = text[prefix.len()..].trim();
        let rest_lower = rest.to_lowercase();

        for except_pat in [" except ", " but ", " other than ", " apart from ", " aside from "] {
            if let Some(idx) = rest_lower.find(except_pat) {
                let agent_part = rest[..idx].trim();
                let except_part_raw = rest[idx + except_pat.len()..].trim();

                // Strip duration suffix from contacts (e.g. "family for 1h" → "family")
                let except_part = strip_duration_suffix(except_part_raw);

                let agent = extract_agent(agent_part);
                let contacts = parse_contacts(except_part);
                let expanded: Vec<String> = contacts.iter()
                    .flat_map(|c| entities.expand(c))
                    .collect();
                if !expanded.is_empty() {
                    return Some((agent, expanded));
                }
            }
        }
    }
    None
}

/// Strip trailing duration like "for 1h" or "until 5pm" from a contact string.
fn strip_duration_suffix(s: &str) -> &str {
    let lower = s.to_lowercase();
    for pat in [" for ", " until ", " lasting "] {
        if let Some(idx) = lower.rfind(pat) {
            return s[..idx].trim();
        }
    }
    s
}

// ── Natural language parsers ───────────────────────────────────────────

/// Extract "... for 1h" or "... for 30 minutes" from end of string.
/// Returns (text_without_duration, Some(duration_string)).
fn extract_duration(text: &str) -> (String, Option<String>) {
    let patterns = [
        " for ",
        " until ",
        " lasting ",
    ];

    for pat in &patterns {
        if let Some(idx) = text.to_lowercase().rfind(pat) {
            let before = text[..idx].trim();
            let after = text[idx + pat.len()..].trim();
            if !after.is_empty() {
                return (before.to_string(), Some(after.to_string()));
            }
        }
    }

    (text.to_string(), None)
}

fn is_mute_all(lower: &str) -> bool {
    let phrases = [
        "mute all", "mute everyone", "mute everything",
        "silence all", "silence everyone", "silence everything",
        "quiet all", "quiet everyone", "quiet everything",
        "suppress all", "suppress everything",
        "do not disturb", "dnd",
    ];
    phrases.iter().any(|p| lower == *p || lower.starts_with(&format!("{} ", p)))
}

fn parse_mute_agent(lower: &str, original: &str) -> Option<String> {
    let prefixes = ["mute ", "silence ", "quiet ", "suppress ", "hide ", "disable "];
    for prefix in &prefixes {
        if lower.starts_with(prefix) {
            let rest = &original[prefix.len()..].trim();
            let agent = extract_agent(rest);
            if !agent.is_empty() && agent != "all" && agent != "everyone" && agent != "everything" {
                return Some(agent);
            }
        }
    }
    None
}

fn parse_surface_agent(lower: &str, original: &str) -> Option<String> {
    let prefixes = ["surface ", "allow ", "unmute ", "enable ", "show "];
    for prefix in &prefixes {
        if lower.starts_with(prefix) {
            let rest = &original[prefix.len()..].trim();
            // Don't match if followed by "from" — that's handled by parse_surface_from
            if rest.to_lowercase().contains(" from ") {
                continue;
            }
            let agent = extract_agent(rest);
            if !agent.is_empty() {
                return Some(agent);
            }
        }
    }
    None
}

fn parse_surface_from(lower: &str, original: &str) -> Option<(String, Vec<String>)> {
    let prefixes = ["surface ", "allow ", "unmute ", "enable ", "show "];
    for prefix in &prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let rest = &original[prefix.len()..].trim();
        let rest_lower = rest.to_lowercase();
        if let Some(from_idx) = rest_lower.find(" from ") {
            let agent = rest[..from_idx].trim().to_string();
            let after_from = rest[from_idx + 6..].trim();
            let contacts = parse_contacts(after_from);
            if !agent.is_empty() && !contacts.is_empty() {
                return Some((agent, contacts));
            }
        }
    }
    None
}

fn parse_only_allow(lower: &str, original: &str) -> Option<(String, Vec<String>)> {
    let prefixes = [
        "only allow ", "only ", "just allow ", "just ",
        "only surface ", "just surface ",
    ];
    for prefix in &prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let rest = &original[prefix.len()..].trim();
        let rest_lower = rest.to_lowercase();
        if let Some(from_idx) = rest_lower.find(" from ") {
            let agent = rest[..from_idx].trim().to_string();
            let after_from = rest[from_idx + 6..].trim();
            let contacts = parse_contacts(after_from);
            if !agent.is_empty() && !contacts.is_empty() {
                return Some((agent, contacts));
            }
        }
    }
    None
}

fn parse_mute_except(lower: &str, original: &str, _entities: &EntityGroups) -> Option<(String, Vec<String>)> {
    let mute_prefixes = ["mute ", "silence ", "quiet ", "suppress "];
    for prefix in &mute_prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let rest = &original[prefix.len()..].trim();
        let rest_lower = rest.to_lowercase();

        // Find "except", "but", "other than" in the remaining text
        for except_pat in [" except ", " but ", " other than ", " apart from ", " aside from "] {
            if let Some(idx) = rest_lower.find(except_pat) {
                let agent_part = rest[..idx].trim();
                let except_part = rest[idx + except_pat.len()..].trim();
                let agent = extract_agent(agent_part);
                let contacts = parse_contacts(except_part);
                if !contacts.is_empty() {
                    return Some((agent, contacts));
                }
            }
        }
    }
    None
}

fn is_urgent_only(lower: &str) -> bool {
    let phrases = [
        "urgent only", "only urgent", "just urgent",
        "high priority only", "only high priority",
        "critical only", "only critical",
    ];
    phrases.iter().any(|p| lower == *p || lower.starts_with(&format!("{} ", p)))
}

fn parse_auto_ack(lower: &str, original: &str) -> Option<String> {
    let prefixes = [
        "auto-ack ", "auto ack ", "autoack ",
        "dismiss ", "auto-dismiss ", "auto dismiss ",
        "auto-answer ", "auto answer ",
    ];
    for prefix in &prefixes {
        if lower.starts_with(prefix) {
            let rest = &original[prefix.len()..].trim();
            let agent = extract_agent(rest);
            if !agent.is_empty() {
                return Some(agent);
            }
        }
    }
    None
}

fn parse_reprioritize(lower: &str, original: &str) -> Option<(String, i32)> {
    let prefixes = [
        "reprioritize ", "set priority ", "set urgency ",
        "change priority ", "change urgency ",
    ];
    for prefix in &prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let rest = &original[prefix.len()..].trim();
        let rest_lower = rest.to_lowercase();

        // Look for "to N" or "as N"
        for to_pat in [" to ", " as ", " at ", " = ", ": "] {
            if let Some(idx) = rest_lower.rfind(to_pat) {
                let agent_part = rest[..idx].trim();
                let urgency_part = rest[idx + to_pat.len()..].trim();
                let agent = extract_agent(agent_part);
                // Parse urgency: "5", "urgent", "high", etc.
                let urgency = parse_urgency(urgency_part);
                if !agent.is_empty() && urgency >= 0 {
                    return Some((agent, urgency));
                }
            }
        }
    }
    None
}

fn parse_urgency(s: &str) -> i32 {
    let lower = s.to_lowercase();
    if let Ok(n) = lower.parse::<i32>() {
        return n.clamp(0, 5);
    }
    match lower.as_str() {
        "critical" | "siren" | "drop everything" => 5,
        "urgent" | "high" => 4,
        "timely" | "medium" => 3,
        "calm" | "normal" | "low" => 2,
        "fyi" | "info" | "background" => 1,
        "silent" | "none" => 0,
        _ => -1, // unrecognized
    }
}

/// Parse a contact list like "Jeff, Carmen and JCS-Central" or "+14155551234, +14155555678"
pub fn parse_contacts(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == '·')
        .map(|p| {
            let trimmed = p.trim();
            // Remove "and" prefix if present
            if trimmed.to_lowercase().starts_with("and ") {
                trimmed[4..].trim().to_string()
            } else {
                trimmed.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Extract agent name from text, skipping filler words.
fn extract_agent(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let skip = ["all", "everything", "everyone", "from", "by", "the", "notifications", "messages", "alerts"];
    for (i, word) in words.iter().enumerate() {
        let lower = word.to_lowercase();
        if !skip.contains(&lower.as_str()) {
            return words[i..].join(" ");
        }
    }
    s.to_string()
}

/// Compile an "allow list" rule that creates two rules:
/// 1. Low-priority mute for the agent (catches everything)
/// 2. High-priority surface rules for each allowed contact (overrides mute)
///
/// Returns (mute_rule, surface_rules) where surface_rules is a Vec.
/// This is meant to be called by the rule creation handler.
pub fn compile_allow_list(
    agent: &str,
    contacts: &[String],
    _duration_str: Option<&str>,
) -> Option<(serde_json::Value, Vec<serde_json::Value>)> {
    let mute_compiled = serde_json::json!({
        "action": "mute",
        "match": { "agent": agent }
    });

    let surface_rules: Vec<serde_json::Value> = contacts
        .iter()
        .map(|contact| {
            serde_json::json!({
                "action": "surface",
                "match": {
                    "agent": agent,
                    "source_contains": [contact.clone()]
                }
            })
        })
        .collect();

    Some((mute_compiled, surface_rules))
}
