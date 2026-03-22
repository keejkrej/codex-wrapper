use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::attachments::{create_attachment_id, parse_base64_data_url, resolve_attachment_path};
use crate::config::ServerConfig;
use crate::model::ReadModelState;
use crate::util::{now_iso, optional_string, required_string, required_string_from_object};

#[derive(Clone)]
pub(crate) struct DecidedEvent {
    pub aggregate_kind: String,
    pub aggregate_id: String,
    pub occurred_at: String,
    pub event_type: String,
    pub payload: Value,
    pub command_id: Option<String>,
    pub causation_event_id: Option<String>,
    pub correlation_id: Option<String>,
    pub metadata: Value,
}

pub(crate) async fn decide(
    snapshot: &ReadModelState,
    config: &ServerConfig,
    command: &Value,
) -> Result<Vec<DecidedEvent>> {
    let command_type = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Command type is required"))?;
    let command_id = command
        .get("commandId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    match command_type {
        "project.create" => Ok(vec![event(
            "project",
            required_string(command, "projectId")?,
            required_string(command, "createdAt")?,
            "project.created",
            json!({
                "projectId": required_string(command, "projectId")?,
                "title": required_string(command, "title")?,
                "workspaceRoot": required_string(command, "workspaceRoot")?,
                "defaultModel": optional_string(command, "defaultModel"),
                "scripts": [],
                "createdAt": required_string(command, "createdAt")?,
                "updatedAt": required_string(command, "createdAt")?,
            }),
            command_id,
        )]),
        "project.meta.update" => {
            let project_id = required_string(command, "projectId")?;
            require_project(snapshot, &project_id)?;
            Ok(vec![event(
                "project",
                project_id.clone(),
                now_iso(),
                "project.meta-updated",
                json!({
                    "projectId": project_id,
                    "title": command.get("title").cloned().unwrap_or(Value::Null),
                    "workspaceRoot": command.get("workspaceRoot").cloned().unwrap_or(Value::Null),
                    "defaultModel": command.get("defaultModel").cloned().unwrap_or(Value::Null),
                    "scripts": command.get("scripts").cloned().unwrap_or(Value::Null),
                    "updatedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "project.delete" => {
            let project_id = required_string(command, "projectId")?;
            require_project(snapshot, &project_id)?;
            Ok(vec![event(
                "project",
                project_id.clone(),
                now_iso(),
                "project.deleted",
                json!({
                    "projectId": project_id,
                    "deletedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "thread.create" => {
            let thread_id = required_string(command, "threadId")?;
            let project_id = required_string(command, "projectId")?;
            require_project(snapshot, &project_id)?;
            let created_at = required_string(command, "createdAt")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.created",
                json!({
                    "threadId": thread_id,
                    "projectId": project_id,
                    "title": required_string(command, "title")?,
                    "model": required_string(command, "model")?,
                    "runtimeMode": required_string(command, "runtimeMode")?,
                    "interactionMode": required_string(command, "interactionMode")?,
                    "branch": command.get("branch").cloned().unwrap_or(Value::Null),
                    "worktreePath": command.get("worktreePath").cloned().unwrap_or(Value::Null),
                    "createdAt": created_at,
                    "updatedAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.meta.update" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                now_iso(),
                "thread.meta-updated",
                json!({
                    "threadId": thread_id,
                    "title": command.get("title").cloned().unwrap_or(Value::Null),
                    "model": command.get("model").cloned().unwrap_or(Value::Null),
                    "branch": command.get("branch").cloned().unwrap_or(Value::Null),
                    "worktreePath": command.get("worktreePath").cloned().unwrap_or(Value::Null),
                    "updatedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "thread.runtime-mode.set" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                now_iso(),
                "thread.runtime-mode-set",
                json!({
                    "threadId": thread_id,
                    "runtimeMode": required_string(command, "runtimeMode")?,
                    "updatedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "thread.interaction-mode.set" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                now_iso(),
                "thread.interaction-mode-set",
                json!({
                    "threadId": thread_id,
                    "interactionMode": required_string(command, "interactionMode")?,
                    "updatedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "thread.delete" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                now_iso(),
                "thread.deleted",
                json!({
                    "threadId": thread_id,
                    "deletedAt": now_iso(),
                }),
                command_id,
            )])
        }
        "thread.turn.start" => decide_turn_start(snapshot, config, command, command_id).await,
        "thread.turn.interrupt" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            let created_at = required_string(command, "createdAt")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.turn-interrupt-requested",
                json!({
                    "threadId": thread_id,
                    "turnId": command.get("turnId").cloned().unwrap_or(Value::Null),
                    "createdAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.approval.respond" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            let created_at = required_string(command, "createdAt")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.approval-response-requested",
                json!({
                    "threadId": thread_id,
                    "requestId": required_string(command, "requestId")?,
                    "decision": required_string(command, "decision")?,
                    "createdAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.user-input.respond" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            let created_at = required_string(command, "createdAt")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.user-input-response-requested",
                json!({
                    "threadId": thread_id,
                    "requestId": required_string(command, "requestId")?,
                    "answers": command.get("answers").cloned().unwrap_or_else(|| json!({})),
                    "createdAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.session.stop" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            let created_at = required_string(command, "createdAt")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.session-stop-requested",
                json!({
                    "threadId": thread_id,
                    "createdAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.checkpoint.revert" => {
            let thread_id = required_string(command, "threadId")?;
            require_thread(snapshot, &thread_id)?;
            let created_at = required_string(command, "createdAt")?;
            let turn_count = command
                .get("turnCount")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Ok(vec![
                event(
                    "thread",
                    thread_id.clone(),
                    created_at.clone(),
                    "thread.checkpoint-revert-requested",
                    json!({
                        "threadId": thread_id,
                        "turnCount": turn_count,
                        "createdAt": created_at,
                    }),
                    command_id.clone(),
                ),
                event(
                    "thread",
                    required_string(command, "threadId")?,
                    now_iso(),
                    "thread.reverted",
                    json!({
                        "threadId": required_string(command, "threadId")?,
                        "turnCount": turn_count,
                    }),
                    command_id,
                ),
            ])
        }
        "thread.session.set" => {
            let thread_id = required_string(command, "threadId")?;
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                command
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(now_iso),
                "thread.session-set",
                json!({
                    "threadId": thread_id,
                    "session": command.get("session").cloned().unwrap_or(Value::Null),
                }),
                command_id,
            )])
        }
        "thread.message.assistant.delta" => {
            let thread_id = required_string(command, "threadId")?;
            let created_at = command
                .get("createdAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(now_iso);
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.message-sent",
                json!({
                    "threadId": thread_id,
                    "messageId": required_string(command, "messageId")?,
                    "role": "assistant",
                    "text": required_string(command, "delta")?,
                    "turnId": command.get("turnId").cloned().unwrap_or(Value::Null),
                    "streaming": true,
                    "createdAt": created_at,
                    "updatedAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.message.assistant.complete" => {
            let thread_id = required_string(command, "threadId")?;
            let created_at = command
                .get("createdAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(now_iso);
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at.clone(),
                "thread.message-sent",
                json!({
                    "threadId": thread_id,
                    "messageId": required_string(command, "messageId")?,
                    "role": "assistant",
                    "text": "",
                    "turnId": command.get("turnId").cloned().unwrap_or(Value::Null),
                    "streaming": false,
                    "createdAt": created_at,
                    "updatedAt": created_at,
                }),
                command_id,
            )])
        }
        "thread.activity.append" => {
            let thread_id = required_string(command, "threadId")?;
            let created_at = command
                .get("createdAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(now_iso);
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at,
                "thread.activity-appended",
                json!({
                    "threadId": thread_id,
                    "activity": command.get("activity").cloned().unwrap_or(Value::Null),
                }),
                command_id,
            )])
        }
        "thread.proposed-plan.upsert" => {
            let thread_id = required_string(command, "threadId")?;
            let created_at = command
                .get("createdAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(now_iso);
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at,
                "thread.proposed-plan-upserted",
                json!({
                    "threadId": thread_id,
                    "proposedPlan": command.get("proposedPlan").cloned().unwrap_or(Value::Null),
                }),
                command_id,
            )])
        }
        "thread.turn.diff.complete" => {
            let thread_id = required_string(command, "threadId")?;
            let created_at = command
                .get("completedAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(now_iso);
            Ok(vec![event(
                "thread",
                thread_id.clone(),
                created_at,
                "thread.turn-diff-completed",
                json!({
                    "threadId": thread_id,
                    "turnId": required_string(command, "turnId")?,
                    "checkpointTurnCount": command.get("checkpointTurnCount").and_then(Value::as_u64).unwrap_or_default(),
                    "checkpointRef": required_string(command, "checkpointRef")?,
                    "status": required_string(command, "status")?,
                    "files": command.get("files").cloned().unwrap_or_else(|| json!([])),
                    "assistantMessageId": command.get("assistantMessageId").cloned().unwrap_or(Value::Null),
                    "completedAt": command
                        .get("completedAt")
                        .cloned()
                        .unwrap_or_else(|| json!(now_iso())),
                }),
                command_id,
            )])
        }
        other => Err(anyhow!("Unsupported orchestration command: {other}")),
    }
}

async fn decide_turn_start(
    snapshot: &ReadModelState,
    config: &ServerConfig,
    command: &Value,
    command_id: Option<String>,
) -> Result<Vec<DecidedEvent>> {
    let thread_id = required_string(command, "threadId")?;
    let thread = require_thread(snapshot, &thread_id)?;
    let message = command
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("message is required"))?;
    let created_at = required_string(command, "createdAt")?;
    let attachments = persist_uploaded_attachments(
        config,
        &thread_id,
        message
            .get("attachments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    )
    .await?;

    Ok(vec![
        event(
            "thread",
            thread_id.clone(),
            created_at.clone(),
            "thread.message-sent",
            json!({
                "threadId": thread_id,
                "messageId": required_string_from_object(message, "messageId")?,
                "role": "user",
                "text": message.get("text").and_then(Value::as_str).unwrap_or_default(),
                "attachments": attachments,
                "turnId": Value::Null,
                "streaming": false,
                "createdAt": created_at,
                "updatedAt": created_at,
            }),
            command_id.clone(),
        ),
        event(
            "thread",
            required_string(command, "threadId")?,
            required_string(command, "createdAt")?,
            "thread.turn-start-requested",
            json!({
                "threadId": required_string(command, "threadId")?,
                "messageId": required_string_from_object(message, "messageId")?,
                "provider": command.get("provider").cloned().unwrap_or(Value::Null),
                "model": command.get("model").cloned().unwrap_or(Value::Null),
                "modelOptions": command.get("modelOptions").cloned().unwrap_or(Value::Null),
                "providerOptions": command.get("providerOptions").cloned().unwrap_or(Value::Null),
                "assistantDeliveryMode": command
                    .get("assistantDeliveryMode")
                    .cloned()
                    .unwrap_or_else(|| json!("buffered")),
                "runtimeMode": json!(thread.runtime_mode),
                "interactionMode": json!(thread.interaction_mode),
                "sourceProposedPlan": command.get("sourceProposedPlan").cloned().unwrap_or(Value::Null),
                "createdAt": required_string(command, "createdAt")?,
            }),
            command_id,
        ),
    ])
}

async fn persist_uploaded_attachments(
    config: &ServerConfig,
    thread_id: &str,
    attachments: Vec<Value>,
) -> Result<Vec<Value>> {
    let mut persisted = Vec::new();
    for attachment in attachments {
        let kind = attachment
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("attachment type is required"))?;
        if kind != "image" {
            continue;
        }
        let name = attachment
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("attachment name is required"))?;
        let data_url = attachment
            .get("dataUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("attachment dataUrl is required"))?;
        let (mime_type, bytes) = parse_base64_data_url(data_url)?;
        if !mime_type.starts_with("image/") || bytes.is_empty() {
            return Err(anyhow!("Invalid image attachment payload for '{name}'"));
        }

        let attachment_id = create_attachment_id(thread_id)
            .ok_or_else(|| anyhow!("Failed to create a safe attachment id"))?;
        let persisted_attachment = crate::model::ChatAttachment {
            kind: "image".to_string(),
            id: attachment_id,
            name: name.to_string(),
            mime_type,
            size_bytes: bytes.len() as u64,
        };
        let attachment_path = resolve_attachment_path(&config.state_dir, &persisted_attachment)
            .ok_or_else(|| anyhow!("Failed to resolve attachment path"))?;
        if let Some(parent) = attachment_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&attachment_path, bytes)?;
        persisted.push(serde_json::to_value(persisted_attachment)?);
    }

    Ok(persisted)
}

fn event(
    aggregate_kind: &str,
    aggregate_id: String,
    occurred_at: String,
    event_type: &str,
    payload: Value,
    command_id: Option<String>,
) -> DecidedEvent {
    DecidedEvent {
        aggregate_kind: aggregate_kind.to_string(),
        aggregate_id,
        occurred_at,
        event_type: event_type.to_string(),
        payload,
        command_id: command_id.clone(),
        causation_event_id: None,
        correlation_id: command_id,
        metadata: json!({}),
    }
}

fn require_project<'a>(
    snapshot: &'a ReadModelState,
    project_id: &str,
) -> Result<&'a crate::model::Project> {
    snapshot
        .projects
        .iter()
        .find(|project| project.id == project_id && project.deleted_at.is_none())
        .ok_or_else(|| anyhow!("Project not found"))
}

fn require_thread<'a>(
    snapshot: &'a ReadModelState,
    thread_id: &str,
) -> Result<&'a crate::model::Thread> {
    snapshot
        .threads
        .iter()
        .find(|thread| thread.id == thread_id && thread.deleted_at.is_none())
        .ok_or_else(|| anyhow!("Thread not found"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::decide;
    use crate::config::ServerConfig;
    use crate::model::{Project, ReadModelState, Thread};
    use crate::util::now_iso;

    #[tokio::test]
    async fn thread_turn_start_emits_user_message_and_request_event() {
        let now = now_iso();
        let read_model = ReadModelState {
            snapshot_sequence: 0,
            projects: vec![Project {
                id: "project-1".to_string(),
                title: "Project".to_string(),
                workspace_root: ".".to_string(),
                default_model: Some("gpt-5-codex".to_string()),
                scripts: vec![],
                created_at: now.clone(),
                updated_at: now.clone(),
                deleted_at: None,
            }],
            threads: vec![Thread {
                id: "thread-1".to_string(),
                project_id: "project-1".to_string(),
                title: "Thread".to_string(),
                model: "gpt-5-codex".to_string(),
                runtime_mode: "full-access".to_string(),
                interaction_mode: "default".to_string(),
                branch: None,
                worktree_path: None,
                latest_turn: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                deleted_at: None,
                messages: vec![],
                proposed_plans: vec![],
                activities: vec![],
                checkpoints: vec![],
                session: None,
            }],
            updated_at: now.clone(),
        };
        let config = ServerConfig::desktop("Test", std::env::current_dir().unwrap());
        let events = decide(
            &read_model,
            &config,
            &json!({
                "type": "thread.turn.start",
                "commandId": "cmd-1",
                "threadId": "thread-1",
                "message": { "messageId": "msg-1", "role": "user", "text": "hello", "attachments": [] },
                "createdAt": now,
            }),
        )
        .await
        .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "thread.message-sent");
        assert_eq!(events[1].event_type, "thread.turn-start-requested");
    }
}
