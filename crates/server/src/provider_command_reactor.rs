use std::sync::Arc;

use serde_json::Value;

use crate::provider_runtime::{ProviderRuntimeService, StartTurnInput};
use crate::state::AppState;
use crate::util::now_iso;

pub(crate) async fn react_to_events(
    state: Arc<AppState>,
    provider_runtime: ProviderRuntimeService,
    events: &[Value],
) {
    for event in events {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = event.get("payload").cloned().unwrap_or(Value::Null);

        match event_type {
            "thread.turn-start-requested" => {
                let provider_name = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| infer_provider_name(&state, &payload));
                provider_runtime
                    .start_turn(
                        state.clone(),
                        StartTurnInput {
                            thread_id: payload["threadId"].as_str().unwrap_or_default().to_string(),
                            provider_name,
                            runtime_mode: payload["runtimeMode"]
                                .as_str()
                                .unwrap_or("full-access")
                                .to_string(),
                            interaction_mode: payload["interactionMode"]
                                .as_str()
                                .unwrap_or("default")
                                .to_string(),
                            prompt: find_user_message_text(&state, &payload)
                                .await
                                .unwrap_or_default(),
                            cwd: find_thread_cwd(&state, &payload)
                                .await
                                .unwrap_or_else(|| state.cwd_string()),
                            state_dir: crate::util::normalize_path(&state.config.state_dir),
                            model: payload
                                .get("model")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            model_options: payload
                                .get("modelOptions")
                                .cloned()
                                .filter(|value| !value.is_null()),
                            provider_options: payload
                                .get("providerOptions")
                                .cloned()
                                .filter(|value| !value.is_null()),
                            assistant_delivery_mode: payload
                                .get("assistantDeliveryMode")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            attachments: find_user_message_attachments(&state, &payload)
                                .await
                                .unwrap_or_default(),
                            created_at: payload["createdAt"]
                                .as_str()
                                .map(ToOwned::to_owned)
                                .unwrap_or_else(now_iso),
                        },
                    )
                    .await;
            }
            "thread.turn-interrupt-requested" => {
                provider_runtime
                    .interrupt_turn(
                        state.clone(),
                        payload["threadId"].as_str().unwrap_or_default(),
                        payload.get("turnId").and_then(Value::as_str),
                    )
                    .await;
            }
            "thread.approval-response-requested" => {
                provider_runtime
                    .respond_to_approval(
                        state.clone(),
                        payload["threadId"].as_str().unwrap_or_default(),
                        payload["requestId"].as_str().unwrap_or_default(),
                        payload["decision"].as_str().unwrap_or_default(),
                    )
                    .await;
            }
            "thread.user-input-response-requested" => {
                provider_runtime
                    .respond_to_user_input(
                        state.clone(),
                        payload["threadId"].as_str().unwrap_or_default(),
                        payload["requestId"].as_str().unwrap_or_default(),
                        payload
                            .get("answers")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({})),
                    )
                    .await;
            }
            "thread.session-stop-requested" => {
                provider_runtime
                    .stop_session(
                        state.clone(),
                        payload["threadId"].as_str().unwrap_or_default(),
                    )
                    .await;
            }
            _ => {}
        }
    }
}

async fn find_user_message_text(state: &AppState, payload: &Value) -> Option<String> {
    let thread_id = payload.get("threadId").and_then(Value::as_str)?;
    let message_id = payload.get("messageId").and_then(Value::as_str)?;
    let snapshot = state.snapshot.lock().await;
    let thread = snapshot
        .threads
        .iter()
        .find(|thread| thread.id == thread_id)?;
    thread
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .map(|message| message.text.clone())
}

async fn find_user_message_attachments(
    state: &AppState,
    payload: &Value,
) -> Option<Vec<crate::model::ChatAttachment>> {
    let thread_id = payload.get("threadId").and_then(Value::as_str)?;
    let message_id = payload.get("messageId").and_then(Value::as_str)?;
    let snapshot = state.snapshot.lock().await;
    let thread = snapshot
        .threads
        .iter()
        .find(|thread| thread.id == thread_id)?;
    thread
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .map(|message| message.attachments.clone())
}

async fn find_thread_cwd(state: &AppState, payload: &Value) -> Option<String> {
    let thread_id = payload.get("threadId").and_then(Value::as_str)?;
    let snapshot = state.snapshot.lock().await;
    let thread = snapshot
        .threads
        .iter()
        .find(|thread| thread.id == thread_id)?;
    thread.worktree_path.clone().or_else(|| {
        snapshot
            .projects
            .iter()
            .find(|project| project.id == thread.project_id)
            .map(|project| project.workspace_root.clone())
    })
}

fn infer_provider_name(state: &AppState, payload: &Value) -> String {
    let thread_id = payload["threadId"].as_str().unwrap_or_default();
    let snapshot = state.snapshot.blocking_lock();
    snapshot
        .threads
        .iter()
        .find(|thread| thread.id == thread_id)
        .and_then(|thread| {
            thread
                .session
                .as_ref()
                .and_then(|session| session.provider_name.clone())
                .or_else(|| {
                    if thread.model.contains("claude") {
                        Some("claudeAgent".to_string())
                    } else {
                        Some("codex".to_string())
                    }
                })
        })
        .unwrap_or_else(|| "codex".to_string())
}
