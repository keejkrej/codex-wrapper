use std::process::Stdio;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::provider_adapter::{
    AdapterEvent, ProviderAdapter, ProviderSessionState, SendTurnInput, StartSessionInput,
};
use crate::provider_runtime_ingestion::RuntimeEvent;
use crate::util::now_iso;

pub(crate) struct ClaudeCliAdapter;

#[async_trait]
impl ProviderAdapter for ClaudeCliAdapter {
    fn provider_name(&self) -> &'static str {
        "claudeAgent"
    }

    async fn start_session(
        &self,
        session: &mut ProviderSessionState,
        _input: &StartSessionInput,
    ) -> Result<()> {
        session.provider_name = self.provider_name().to_string();
        if session.provider_session_id.is_none() {
            session.provider_session_id = Some(uuid::Uuid::new_v4().to_string());
        }
        Ok(())
    }

    async fn send_turn(
        &self,
        session: ProviderSessionState,
        input: SendTurnInput,
        events: mpsc::UnboundedSender<AdapterEvent>,
        mut kill: oneshot::Receiver<()>,
    ) -> Result<ProviderSessionState> {
        let session_id = session
            .provider_session_id
            .clone()
            .ok_or_else(|| anyhow!("claude session id is not initialized"))?;

        let mut command = Command::new("claude");
        command
            .arg("-p")
            .arg("--verbose")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--session-id")
            .arg(&session_id)
            .arg("--permission-mode")
            .arg(if input.runtime_mode == "full-access" {
                "bypassPermissions"
            } else {
                "default"
            })
            .arg("--model");
        command.arg(
            input
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
        );
        command.arg(build_prompt_with_context(&input));
        command
            .current_dir(&input.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|error| anyhow!("failed to spawn claude: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("claude stdout was unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("claude stderr was unavailable"))?;

        let stderr_task = tokio::spawn(read_stderr(stderr));
        let read_task = tokio::spawn(read_claude_stdout(
            stdout,
            input.thread_id.clone(),
            input.turn_id.clone(),
            input.assistant_message_id.clone(),
            events.clone(),
        ));

        let status = tokio::select! {
            status = child.wait() => status.map_err(|error| anyhow!("failed to wait for claude: {error}"))?,
            _ = &mut kill => {
                let _ = child.kill().await;
                return Err(anyhow!("claude turn interrupted"));
            }
        };
        let read_result = read_task
            .await
            .map_err(|error| anyhow!("failed to join claude stdout task: {error}"))??;
        let stderr_output = stderr_task.await.unwrap_or_default();

        if !status.success() || read_result.is_error {
            return Err(anyhow!(if read_result.error_message.trim().is_empty() {
                if stderr_output.trim().is_empty() {
                    "claude command failed".to_string()
                } else {
                    stderr_output
                }
            } else {
                read_result.error_message
            }));
        }

        Ok(session)
    }
}

struct ClaudeReadResult {
    is_error: bool,
    error_message: String,
}

async fn read_claude_stdout(
    stdout: impl AsyncRead + Unpin,
    thread_id: String,
    turn_id: String,
    assistant_message_id: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
) -> Result<ClaudeReadResult> {
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_running = false;
    let mut error_message = String::new();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };
        match value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "system" => {
                if !saw_running {
                    saw_running = true;
                    let _ = events.send(AdapterEvent::SessionId(
                        value
                            .get("session_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ));
                    let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::SessionSet {
                        thread_id: thread_id.clone(),
                        status: "running".to_string(),
                        provider_name: "claudeAgent".to_string(),
                        runtime_mode: "full-access".to_string(),
                        active_turn_id: Some(turn_id.clone()),
                        last_error: None,
                        updated_at: now_iso(),
                    }));
                }
            }
            "assistant" => {
                let message = value.get("message").cloned().unwrap_or_default();
                let content = message
                    .get("content")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let text = content
                    .iter()
                    .filter_map(|entry| entry.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::AssistantDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id: assistant_message_id.clone(),
                        delta: text,
                        created_at: now_iso(),
                    }));
                }
                if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
                    error_message = error.to_string();
                }
            }
            "result" => {
                let is_error = value
                    .get("is_error")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if is_error {
                    error_message = value
                        .get("result")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("claude command failed")
                        .to_string();
                } else {
                    let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::AssistantComplete {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id: assistant_message_id.clone(),
                        created_at: now_iso(),
                    }));
                }
                return Ok(ClaudeReadResult {
                    is_error,
                    error_message,
                });
            }
            _ => {}
        }
    }

    Ok(ClaudeReadResult {
        is_error: !error_message.is_empty(),
        error_message,
    })
}

async fn read_stderr(stderr: impl AsyncRead + Unpin) -> String {
    let mut lines = BufReader::new(stderr).lines();
    let mut output = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&line);
    }
    output
}

fn build_prompt_with_context(input: &SendTurnInput) -> String {
    let mut prompt = input.prompt.clone();
    if input.interaction_mode == "plan" {
        prompt = format!("Start with a concise plan, then continue.\n\n{prompt}");
    }
    if input.model_options.is_some() || input.provider_options.is_some() {
        prompt.push_str("\n\nRuntime options were supplied for this turn.");
    }
    if input.attachments.is_empty() {
        return prompt;
    }
    let attachment_list = input
        .attachments
        .iter()
        .map(|attachment| format!("- {} ({})", attachment.name, attachment.id))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{prompt}\n\nAttachment file identifiers available in the workspace state:\n{attachment_list}"
    )
}
