use crate::models::*;
use anyhow::Result;
use chrono::Utc;

/// Evaluate rules against a notification. Returns the modified notification and
/// optionally an auto-answer value.
pub fn evaluate(rules: &[Rule], mut notif: Notification) -> (Notification, Option<String>) {
    let now = Utc::now();
    let mut auto_answer: Option<String> = None;

    for rule in rules {
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
                a if a.starts_with("reprioritize:") => {
                    if let Some(val) = a.strip_prefix("reprioritize:").and_then(|s| s.parse::<i32>().ok()) {
                        notif.urgency = val.clamp(0, 5);
                    }
                }
                a if a.starts_with("snooze:") => {
                    // Parse duration like "15m", "1h"
                    if let Some(dur_str) = a.strip_prefix("snooze:") {
                        if let Ok(d) = crate::models::parse_deadline(dur_str) {
                            notif.deadline = Some(d);
                        }
                    }
                }
                a if a.starts_with("auto_answer:") => {
                    auto_answer = a.strip_prefix("auto_answer:").map(|s| s.to_string());
                }
                "surface" | _ => {
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

    // Check question_includes
    if let Some(substr) = match_obj.get("question_includes").and_then(|v| v.as_str()) {
        if !notif.question.to_lowercase().contains(&substr.to_lowercase()) {
            return false;
        }
    }

    // Check source match
    if let Some(source_val) = match_obj.get("source") {
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

    true
}
