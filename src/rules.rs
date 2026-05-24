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
/// Supported patterns:
/// - "mute [agent]" → action: mute, match: { agent }
/// - "mute [agent] for [duration]" → action: mute, match: { agent }, expires_at
/// - "allow [agent] from [contact1, contact2]" → creates compiled allow rule
/// - "only allow [agent] from [contact1, contact2] for [duration]" → creates allow + mute-all
/// - "surface [agent] from [contact]" → action: surface
/// - "reprioritize [agent] to [0-5]" → action: reprioritize:N
pub fn compile(text: &str) -> Option<serde_json::Value> {
    let lower = text.to_lowercase();

    // Pattern: "mute <agent>" or "mute all <agent>" or "mute everything from <agent>"
    if lower.starts_with("mute ") {
        let rest = &text[5..].trim();
        let agent = extract_agent(rest);
        let compiled = serde_json::json!({
            "action": "mute",
            "match": { "agent": agent }
        });
        return Some(compiled);
    }

    // Pattern: "surface <agent> from <contact>" or "allow <agent> from <contact>"
    if lower.starts_with("surface ") || lower.starts_with("allow ") || lower.starts_with("unmute ") {
        let rest = text.splitn(2, ' ').nth(1)?;
        let rest_lower = rest.to_lowercase();

        // Extract agent name (first word or phrase before "from")
        let (agent, contact_part) = if let Some(idx) = rest_lower.find(" from ") {
            (rest[..idx].trim().to_string(), rest[idx + 6..].trim())
        } else {
            (rest.trim().to_string(), "")
        };

        if contact_part.is_empty() {
            // No contact specified — surface everything from this agent
            return Some(serde_json::json!({
                "action": "surface",
                "match": { "agent": agent }
            }));
        }

        // Parse contacts (comma-separated or "and"-separated)
        let contacts = parse_contacts(contact_part);

        // Build source matching
        // For iMessage/Signal, source is "Agent · Handle" or "Agent · +1234567890"
        // We match on the handle part using source_contains
        let compiled = serde_json::json!({
            "action": "surface",
            "match": {
                "agent": agent,
                "source_contains": contacts
            }
        });
        return Some(compiled);
    }

    // Pattern: "only allow <agent> from <contacts>" → this is a meta-pattern handled by the caller
    // (creates mute + surface rules)

    None
}

/// Parse a contact list like "Jeff, Carmen and JCS-Central" or "+14155551234, +14155555678"
pub fn parse_contacts(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == '·')
        .map(|p| {
            let trimmed = p.trim();
            // Remove "and" prefix if present
            if trimmed.starts_with("and ") {
                trimmed[4..].trim().to_string()
            } else {
                trimmed.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Extract agent name from text like "all iMessage notifications" → "iMessage"
fn extract_agent(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    // Skip common filler words at start
    let skip = ["all", "everything", "from", "by"];
    for (i, word) in words.iter().enumerate() {
        if !skip.contains(&word.to_lowercase().as_str()) {
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
