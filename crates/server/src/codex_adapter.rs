use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::attachments::resolve_attachment_path;
use crate::provider_adapter::{
    AdapterEvent, ProviderAdapter, ProviderSessionState, SendTurnInput, StartSessionInput,
};
use crate::provider_runtime_ingestion::RuntimeEvent;
use crate::util::now_iso;

pub(crate) struct CodexCliAdapter;

#[async_trait]
impl ProviderAdapter for CodexCliAdapter {
    fn provider_name(&self) -> &'static str {
        "codex"
    }

    async fn start_session(
        &self,
        session: &mut ProviderSessionState,
        _input: &StartSessionInput,
    ) -> Result<()> {
        session.provider_name = self.provider_name().to_string();
        Ok(())
    }

    async fn send_turn(
        &self,
        mut session: ProviderSessionState,
        input: SendTurnInput,
        events: mpsc::UnboundedSender<AdapterEvent>,
        mut kill: oneshot::Receiver<()>,
    ) -> Result<ProviderSessionState> {
        let mut args = Vec::<String>::new();
        args.push("exec".to_string());
        if let Some(session_id) = session.provider_session_id.as_deref() {
            args.push("resume".to_string());
            args.push(session_id.to_string());
        }
        args.push("--json".to_string());
        args.push("--skip-git-repo-check".to_string());
        args.push("-C".to_string());
        args.push(input.cwd.clone());
        if let Some(model) = input.model.as_deref() {
            args.push("-m".to_string());
            args.push(model.to_string());
        }
        if input.runtime_mode == "full-access" {
            args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
        } else {
            args.push("--full-auto".to_string());
        }
        for attachment in &input.attachments {
            if let Some(path) =
                resolve_attachment_path(PathBuf::from(&input.state_dir).as_path(), attachment)
            {
                args.push("-i".to_string());
                args.push(path.to_string_lossy().to_string());
            }
        }
        args.push(build_prompt_with_context(&input));

        let mut command = Command::new("codex");
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|error| anyhow!("failed to spawn codex: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("codex stdout was unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("codex stderr was unavailable"))?;

        let stderr_task = tokio::spawn(read_stderr(stderr));
        let read_task = tokio::spawn(read_codex_stdout(
            stdout,
            input.thread_id.clone(),
            input.turn_id.clone(),
            input.assistant_message_id.clone(),
            events.clone(),
        ));

        let status = tokio::select! {
            status = child.wait() => status.map_err(|error| anyhow!("failed to wait for codex: {error}"))?,
            _ = &mut kill => {
                let _ = child.kill().await;
                return Err(anyhow!("codex turn interrupted"));
            }
        };
        let _ = read_task.await;
        let stderr_output = stderr_task.await.unwrap_or_default();

        if !status.success() {
            return Err(anyhow!(if stderr_output.trim().is_empty() {
                "codex command failed".to_string()
            } else {
                stderr_output
            }));
        }

        if let Some(thread_id) = read_codex_thread_id(&stderr_output) {
            session.provider_session_id = Some(thread_id);
        }

        Ok(session)
    }
}

async fn read_codex_stdout(
    stdout: impl AsyncRead + Unpin,
    thread_id: String,
    turn_id: String,
    assistant_message_id: String,
    events: mpsc::UnboundedSender<AdapterEvent>,
) -> Result<()> {
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_assistant_output = false;
    let mut provider_thread_id: Option<String> = None;

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
            "thread.started" => {
                if let Some(provider_id) =
                    value.get("thread_id").and_then(serde_json::Value::as_str)
                {
                    provider_thread_id = Some(provider_id.to_string());
                    let _ = events.send(AdapterEvent::SessionId(provider_id.to_string()));
                }
            }
            "turn.started" => {
                let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::SessionSet {
                    thread_id: thread_id.clone(),
                    status: "running".to_string(),
                    provider_name: "codex".to_string(),
                    runtime_mode: "full-access".to_string(),
                    active_turn_id: Some(turn_id.clone()),
                    last_error: None,
                    updated_at: now_iso(),
                }));
            }
            "item.completed" => {
                let item = value.get("item").cloned().unwrap_or_default();
                if item.get("type").and_then(serde_json::Value::as_str) == Some("agent_message") {
                    let text = item
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if !text.is_empty() {
                        saw_assistant_output = true;
                        let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::AssistantDelta {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            message_id: assistant_message_id.clone(),
                            delta: text,
                            created_at: now_iso(),
                        }));
                    }
                }
            }
            "turn.completed" => {
                if saw_assistant_output {
                    let _ = events.send(AdapterEvent::Runtime(RuntimeEvent::AssistantComplete {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        message_id: assistant_message_id.clone(),
                        created_at: now_iso(),
                    }));
                }
            }
            _ => {}
        }
    }

    if let Some(provider_id) = provider_thread_id {
        let _ = events.send(AdapterEvent::SessionId(provider_id));
    }
    Ok(())
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

fn read_codex_thread_id(stderr_output: &str) -> Option<String> {
    stderr_output.lines().find_map(|line| {
        line.strip_prefix("session id: ")
            .map(|value| value.trim().to_string())
    })
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
    format!("{prompt}\n\nImage attachments:\n{attachment_list}")
}
