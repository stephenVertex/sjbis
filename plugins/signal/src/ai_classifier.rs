use anyhow::{Context, Result};
use serde_json::json;

const DEFAULT_MODEL: &str = "accounts/fireworks/models/kimi-k2p6";

/// Ask the AI whether this Signal message is a direct question requiring a human response.
/// Returns (is_question, explanation).
pub async fn is_question_for_user(text: &str, from: &str) -> Result<(bool, String)> {
    let api_key = std::env::var("FIREWORKS_API_KEY")
        .or_else(|_| std::env::var("SJBIS_FIREWORKS_KEY"))
        .context("No Fireworks API key found. Set FIREWORKS_API_KEY or SJBIS_FIREWORKS_KEY.")?;

    let client = reqwest::Client::new();

    let system_prompt = r#"You are a triage assistant for a notification system.
Your job: determine if an incoming Signal message is a direct question that requires a human response.

Output ONLY a JSON object with these fields:
- is_question: boolean — true if the message contains a direct question for the recipient
- confidence: number 0-1
- explanation: string — brief reason for your decision

Rules:
- Group chat announcements, spam, and automated messages are NOT questions
- Only mark as question if a real person is asking the recipient something specific
- "Want to grab lunch?" → true
- "What time works?" → true
- "Are you free tomorrow?" → true
- "Please confirm this" → true
- "Let me know what you think" → true
- "Hey, how are you?" (just greeting) → false
- "Check out this link" (no question) → false
- Random messages with no question → false

Be conservative. When in doubt, false is better than surfacing spam."#;

    let user_prompt = format!(
        r#"From: {}
Message: {}

Is this a direct question requiring a human response?"#,
        from, text
    );

    let body = json!({
        "model": DEFAULT_MODEL,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "temperature": 0.0,
        "max_tokens": 256,
        "response_format": { "type": "json_object" }
    });

    let resp = client
        .post("https://api.fireworks.ai/inference/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to call Fireworks API")?;

    let status = resp.status();
    let resp_json: serde_json::Value = resp.json().await
        .context("Failed to parse Fireworks response")?;

    if !status.is_success() {
        anyhow::bail!("Fireworks API error ({}): {:?}", status, resp_json);
    }

    let content = resp_json["choices"][0]["message"]["content"]
        .as_str()
        .context("Missing content in Fireworks response")?;

    let parsed: serde_json::Value = serde_json::from_str(content)
        .with_context(|| format!("Failed to parse AI response JSON: {}", content))?;

    let is_question = parsed["is_question"]
        .as_bool()
        .unwrap_or(false);

    let explanation = parsed["explanation"]
        .as_str()
        .unwrap_or("no explanation")
        .to_string();

    Ok((is_question, explanation))
}
