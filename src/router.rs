use crate::models::*;
use anyhow::{Context, Result};
use serde_json::json;

const DEFAULT_MODEL: &str = "fireworks-ai/accounts/fireworks/models/kimi-k2p6";

pub struct AiRouter {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl AiRouter {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    /// Call the AI router for classification, urgency prediction, and dedupe detection.
    /// Returns a RouterResult. On failure, returns a default result so the system stays deterministic.
    pub async fn classify(
        &self,
        question: &str,
        agent_name: &str,
        caller_urgency: i32,
        open_notifications: &[Notification],
    ) -> RouterResult {
        // Build the prompt exactly as specified in the design doc
        let open_notifs_json = open_notifications.iter().map(|n| {
            json!({
                "id": n.id,
                "source": n.src,
                "question": n.question,
            })
        }).collect::<Vec<_>>();

        let system_prompt = r#"You triage notifications headed for Stephen's information surfacer.
Output JSON with fields:
  urgency_predicted  (0..5)        — what you'd set if the caller hadn't
  renderer_suggested (one of: yesno|multichoice|freetext|numeric|file|diff|ack|picklist|schedule)
  looks_like_id      (id of an open notification this duplicates, or null)
  confidence         (0..1)

Do not editorialize. Do not rewrite the question."#;

        let user_prompt = format!(
            r#"question: "{}"
caller_source: "{}"
caller_urgency: {}
open_notifications:
{}
"#,
            question,
            agent_name,
            caller_urgency,
            serde_json::to_string_pretty(&open_notifs_json).unwrap_or_default()
        );

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "temperature": 0.0,
            "max_tokens": 256,
            "response_format": { "type": "json_object" }
        });

        match self.call_fireworks(body).await {
            Ok(result) => {
                tracing::info!("router classified: {:?}", result);
                result
            }
            Err(e) => {
                tracing::warn!("AI router failed, falling back: {}", e);
                RouterResult {
                    urgency_predicted: Some(caller_urgency),
                    renderer_suggested: None,
                    looks_like_id: None,
                    confidence: Some(0.0),
                    renderer_guessed: false,
                }
            }
        }
    }

    /// Compile a natural-language rule into a JSON filter.
    pub async fn compile_rule(&self, rule_text: &str, now: chrono::DateTime<chrono::Utc>, agents: &[Agent]) -> Result<serde_json::Value> {
        let known_agents = agents.iter().map(|a| &a.name).collect::<Vec<_>>();
        let system_prompt = r#"Convert one English rule into a JSON filter. Schema:
  {
    match:          { source?, agent?, urgency_min?, urgency_max?, question_includes? },
    action:         "surface" | "mute" | "snooze:<duration>" | "auto_answer:<value>" | "reprioritize:<0-5>",
    expires_at:     ISO-8601 | null,            // one-shot rules use this
    active_window:  { start: "HH:MM", end: "HH:MM", tz: "..." } | null  // daily recurring
  }
Use the user's known agents (provided). Always emit a full ISO-8601 timestamp for one-shot rules — never a bare HH:MM. If the time is ambiguous (e.g. "at 3pm" without "today"), return { error: "ambiguous_time", suggestions: ["today 15:00", "tomorrow 15:00", "every day at 15:00"] }."#;

        let user_prompt = format!(
            r#"now: {}
known_agents: {:?}
rule: "{}"
"#,
            now.to_rfc3339(),
            known_agents,
            rule_text
        );

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "temperature": 0.0,
            "max_tokens": 512,
            "response_format": { "type": "json_object" }
        });

        let text = self.call_fireworks_raw(body).await?;
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse compiled rule JSON: {}", text))?;
        Ok(parsed)
    }

    /// Guess the renderer when no explicit answer shape is given.
    pub async fn guess_renderer(&self, question: &str) -> RouterResult {
        let system_prompt = r#"Pick the best renderer for this question. Output JSON with:
  renderer_suggested (one of: yesno|multichoice|freetext|numeric|file|diff|ack|picklist|schedule)
  confidence (0..1)
  urgency_predicted (0..5, estimate)"#;

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": question }
            ],
            "temperature": 0.0,
            "max_tokens": 128,
            "response_format": { "type": "json_object" }
        });

        match self.call_fireworks(body).await {
            Ok(result) => {
                RouterResult {
                    renderer_guessed: true,
                    ..result
                }
            }
            Err(e) => {
                tracing::warn!("renderer guess failed: {}", e);
                RouterResult {
                    renderer_guessed: true,
                    renderer_suggested: Some("ack".to_string()),
                    confidence: Some(0.0),
                    urgency_predicted: Some(2),
                    looks_like_id: None,
                }
            }
        }
    }

    async fn call_fireworks(&self, body: serde_json::Value) -> Result<RouterResult> {
        let text = self.call_fireworks_raw(body).await?;
        let parsed: RouterResult = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse router result: {}", text))?;
        Ok(parsed)
    }

    async fn call_fireworks_raw(&self, body: serde_json::Value) -> Result<String> {
        let resp = self.client
            .post("https://api.fireworks.ai/inference/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("failed to call Fireworks API")?;

        let status = resp.status();
        let resp_json: serde_json::Value = resp.json().await
            .context("failed to parse Fireworks response")?;

        if !status.is_success() {
            anyhow::bail!("Fireworks API error ({}): {:?}", status, resp_json);
        }

        let content = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .context("missing content in Fireworks response")?
            .to_string();
        Ok(content)
    }
}
