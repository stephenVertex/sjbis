use std::process::Stdio;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::error;

/// A fully specified `sjbis ask` process invocation.
///
/// The question content is intentionally stored separately from `arguments`:
/// `sjbis` reads it from stdin when `--content-stdin` is present, keeping
/// message bodies out of process inspection.
#[derive(Debug, PartialEq, Eq)]
struct AskInvocation {
    program: String,
    arguments: Vec<String>,
    stdin_payload: Vec<u8>,
}

#[derive(Serialize)]
struct AskContent<'a> {
    question: &'a str,
    detail: &'a str,
}

impl AskInvocation {
    fn spawn(&self) -> std::io::Result<tokio::process::Child> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn()
    }

    #[cfg(test)]
    fn argv(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.program.as_str()).chain(self.arguments.iter().map(String::as_str))
    }
}

/// Build the `sjbis ask` invocation without placing question content in argv.
fn build_ask_invocation(
    sjbis_binary: &str,
    agent_name: &str,
    profile: &str,
    source: &str,
    question: &str,
    detail: &str,
) -> Result<AskInvocation> {
    let stdin_payload = serde_json::to_vec(&AskContent { question, detail })
        .context("Failed to serialize sjbis ask stdin content")?;

    Ok(AskInvocation {
        program: sjbis_binary.to_string(),
        arguments: vec![
            "ask".to_string(),
            "--content-stdin".to_string(),
            "--yesno".to_string(),
            "--blocking".to_string(),
            "--json".to_string(),
            "--agent-name".to_string(),
            agent_name.to_string(),
            "--instance".to_string(),
            format!("{} · {}", profile, source),
        ],
        stdin_payload,
    })
}

/// Surface a question via `sjbis ask` CLI.
pub(crate) async fn surface_question(
    sjbis_binary: &str,
    agent_name: &str,
    profile: &str,
    source: &str,
    question: &str,
    detail: &str,
) -> Result<Option<String>> {
    let invocation =
        build_ask_invocation(sjbis_binary, agent_name, profile, source, question, detail)?;
    let mut child = invocation.spawn().context("Failed to run sjbis ask")?;

    let mut stdin = child
        .stdin
        .take()
        .context("sjbis ask stdin was not piped")?;
    stdin
        .write_all(&invocation.stdin_payload)
        .await
        .context("Failed to write sjbis ask stdin content")?;
    stdin
        .shutdown()
        .await
        .context("Failed to close sjbis ask stdin")?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .context("Failed to wait for sjbis ask")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("sjbis ask failed: {}", stderr);
        return Err(anyhow::anyhow!("sjbis ask failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse sjbis response")?;

    Ok(response
        .get("answer")
        .and_then(|value| value.as_str())
        .map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_invocation_keeps_message_content_off_argv() {
        const BODY_SENTINEL: &str = "GMAIL_BODY_SENTINEL";
        const LINK_SENTINEL: &str = "https://mail.example.test/review?message=unique-link";
        const TOKEN_SENTINEL: &str = "secret-token-123";
        let question = format!("{BODY_SENTINEL}: approve {LINK_SENTINEL}");
        let detail = format!("CHAT_DETAIL_SENTINEL: token={TOKEN_SENTINEL}");

        let invocation = build_ask_invocation(
            "sjbis",
            "Gog",
            "work",
            "sender@example.test",
            &question,
            &detail,
        )
        .expect("stdin payload should serialize");

        let argv: Vec<_> = invocation.argv().collect();
        assert!(argv.contains(&"--content-stdin"));
        assert!(!argv.contains(&"--question"));
        assert!(!argv.contains(&"--detail"));
        for argument in &argv {
            assert!(
                !argument.contains(BODY_SENTINEL),
                "question body leaked into argv: {argument:?}"
            );
            assert!(
                !argument.contains(LINK_SENTINEL),
                "message link leaked into argv: {argument:?}"
            );
            assert!(
                !argument.contains(TOKEN_SENTINEL),
                "message token leaked into argv: {argument:?}"
            );
        }

        let stdin: serde_json::Value = serde_json::from_slice(&invocation.stdin_payload)
            .expect("stdin payload should be valid JSON");
        assert_eq!(
            stdin,
            serde_json::json!({
                "question": question,
                "detail": detail,
            })
        );
    }
}
