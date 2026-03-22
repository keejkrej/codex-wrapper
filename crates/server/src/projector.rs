use serde_json::Value;

use crate::model::{
    LatestTurn, Project, ReadModelState, Thread, ThreadActivity, ThreadMessage, ThreadSession,
};

pub(crate) fn apply(snapshot: &mut ReadModelState, event: &Value) {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload = event.get("payload").cloned().unwrap_or(Value::Null);
    let occurred_at = event
        .get("occurredAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    match event_type {
        "project.created" => {
            snapshot
                .projects
                .retain(|project| project.id != payload["projectId"]);
            snapshot.projects.push(Project {
                id: payload["projectId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                title: payload["title"].as_str().unwrap_or_default().to_string(),
                workspace_root: payload["workspaceRoot"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                default_model: payload["defaultModel"].as_str().map(ToOwned::to_owned),
                scripts: Vec::new(),
                created_at: payload["createdAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                updated_at: payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                deleted_at: None,
            });
        }
        "project.meta-updated" => {
            if let Some(project) = snapshot
                .projects
                .iter_mut()
                .find(|project| project.id == payload["projectId"].as_str().unwrap_or_default())
            {
                if let Some(title) = payload["title"].as_str() {
                    project.title = title.to_string();
                }
                if let Some(workspace_root) = payload["workspaceRoot"].as_str() {
                    project.workspace_root = workspace_root.to_string();
                }
                if !payload["defaultModel"].is_null() {
                    project.default_model = payload["defaultModel"].as_str().map(ToOwned::to_owned);
                }
                project.updated_at = payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "project.deleted" => {
            if let Some(project) = snapshot
                .projects
                .iter_mut()
                .find(|project| project.id == payload["projectId"].as_str().unwrap_or_default())
            {
                project.deleted_at = payload["deletedAt"].as_str().map(ToOwned::to_owned);
                project.updated_at = payload["deletedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "thread.created" => {
            snapshot
                .threads
                .retain(|thread| thread.id != payload["threadId"]);
            snapshot.threads.push(Thread {
                id: payload["threadId"].as_str().unwrap_or_default().to_string(),
                project_id: payload["projectId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                title: payload["title"].as_str().unwrap_or_default().to_string(),
                model: payload["model"].as_str().unwrap_or_default().to_string(),
                runtime_mode: payload["runtimeMode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                interaction_mode: payload["interactionMode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                branch: payload["branch"].as_str().map(ToOwned::to_owned),
                worktree_path: payload["worktreePath"].as_str().map(ToOwned::to_owned),
                latest_turn: None,
                created_at: payload["createdAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                updated_at: payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                deleted_at: None,
                messages: Vec::new(),
                proposed_plans: Vec::new(),
                activities: Vec::new(),
                checkpoints: Vec::new(),
                session: None,
            });
        }
        "thread.meta-updated" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                if let Some(title) = payload["title"].as_str() {
                    thread.title = title.to_string();
                }
                if let Some(model) = payload["model"].as_str() {
                    thread.model = model.to_string();
                }
                if !payload["branch"].is_null() {
                    thread.branch = payload["branch"].as_str().map(ToOwned::to_owned);
                }
                if !payload["worktreePath"].is_null() {
                    thread.worktree_path = payload["worktreePath"].as_str().map(ToOwned::to_owned);
                }
                thread.updated_at = payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "thread.runtime-mode-set" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                thread.runtime_mode = payload["runtimeMode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                thread.updated_at = payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "thread.interaction-mode-set" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                thread.interaction_mode = payload["interactionMode"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                thread.updated_at = payload["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "thread.deleted" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                thread.deleted_at = payload["deletedAt"].as_str().map(ToOwned::to_owned);
                thread.updated_at = payload["deletedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
        "thread.message-sent" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                let message_id = payload["messageId"].as_str().unwrap_or_default();
                let existing_index = thread
                    .messages
                    .iter()
                    .position(|message| message.id == message_id);
                if let Some(index) = existing_index {
                    let existing = &mut thread.messages[index];
                    let next_text = payload["text"].as_str().unwrap_or_default();
                    existing.text = if payload["streaming"].as_bool().unwrap_or(false) {
                        format!("{}{}", existing.text, next_text)
                    } else if next_text.is_empty() {
                        existing.text.clone()
                    } else {
                        next_text.to_string()
                    };
                    existing.streaming = payload["streaming"].as_bool().unwrap_or(false);
                    existing.updated_at = payload["updatedAt"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    existing.turn_id = payload["turnId"].as_str().map(ToOwned::to_owned);
                    if let Some(attachments) = payload["attachments"].as_array() {
                        existing.attachments = attachments
                            .iter()
                            .filter_map(|attachment| {
                                serde_json::from_value(attachment.clone()).ok()
                            })
                            .collect();
                    }
                } else {
                    thread.messages.push(ThreadMessage {
                        id: message_id.to_string(),
                        role: payload["role"].as_str().unwrap_or_default().to_string(),
                        text: payload["text"].as_str().unwrap_or_default().to_string(),
                        attachments: payload["attachments"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|attachment| serde_json::from_value(attachment).ok())
                            .collect(),
                        turn_id: payload["turnId"].as_str().map(ToOwned::to_owned),
                        streaming: payload["streaming"].as_bool().unwrap_or(false),
                        created_at: payload["createdAt"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        updated_at: payload["updatedAt"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    });
                }
                thread.updated_at = occurred_at.clone();
            }
        }
        "thread.session-set" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                let session = payload["session"].clone();
                let active_turn_id = session["activeTurnId"].as_str().map(ToOwned::to_owned);
                let session_status = session["status"].as_str().unwrap_or_default().to_string();
                let session_updated_at = session["updatedAt"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                thread.session = Some(ThreadSession {
                    thread_id: payload["threadId"].as_str().unwrap_or_default().to_string(),
                    status: session_status.clone(),
                    provider_name: session["providerName"].as_str().map(ToOwned::to_owned),
                    runtime_mode: session["runtimeMode"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    active_turn_id: active_turn_id.clone(),
                    last_error: session["lastError"].as_str().map(ToOwned::to_owned),
                    updated_at: session_updated_at.clone(),
                });

                if session_status == "running" {
                    if let Some(turn_id) = active_turn_id {
                        let previous = thread.latest_turn.clone();
                        thread.latest_turn = Some(LatestTurn {
                            turn_id: turn_id.clone(),
                            state: "running".to_string(),
                            requested_at: previous
                                .as_ref()
                                .filter(|entry| entry.turn_id == turn_id)
                                .map(|entry| entry.requested_at.clone())
                                .unwrap_or_else(|| session_updated_at.clone()),
                            started_at: Some(
                                previous
                                    .as_ref()
                                    .filter(|entry| entry.turn_id == turn_id)
                                    .and_then(|entry| entry.started_at.clone())
                                    .unwrap_or_else(|| session_updated_at.clone()),
                            ),
                            completed_at: None,
                            assistant_message_id: previous
                                .as_ref()
                                .filter(|entry| entry.turn_id == turn_id)
                                .and_then(|entry| entry.assistant_message_id.clone()),
                        });
                    }
                } else if session_status == "interrupted" {
                    if let Some(latest_turn) = thread.latest_turn.as_mut() {
                        latest_turn.state = "interrupted".to_string();
                        latest_turn.completed_at = Some(session_updated_at.clone());
                    }
                }
                thread.updated_at = occurred_at.clone();
            }
        }
        "thread.activity-appended" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                if let Some(activity_value) = payload.get("activity") {
                    let activity = ThreadActivity {
                        id: activity_value["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        tone: activity_value["tone"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        kind: activity_value["kind"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        summary: activity_value["summary"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        payload: activity_value["payload"].clone(),
                        turn_id: activity_value["turnId"].as_str().map(ToOwned::to_owned),
                        sequence: activity_value["sequence"].as_u64(),
                        created_at: activity_value["createdAt"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    };
                    thread.activities.retain(|entry| entry.id != activity.id);
                    thread.activities.push(activity);
                    thread.activities.sort_by(|left, right| {
                        left.created_at
                            .cmp(&right.created_at)
                            .then_with(|| left.id.cmp(&right.id))
                    });
                }
                thread.updated_at = occurred_at.clone();
            }
        }
        "thread.proposed-plan-upserted" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                if let Some(proposed_plan) = payload.get("proposedPlan") {
                    let id = proposed_plan["id"].as_str().unwrap_or_default();
                    thread.proposed_plans.retain(|entry| entry["id"] != id);
                    thread.proposed_plans.push(proposed_plan.clone());
                    thread.proposed_plans.sort_by(|left, right| {
                        left["createdAt"]
                            .as_str()
                            .unwrap_or_default()
                            .cmp(right["createdAt"].as_str().unwrap_or_default())
                            .then_with(|| {
                                left["id"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .cmp(right["id"].as_str().unwrap_or_default())
                            })
                    });
                }
                thread.updated_at = occurred_at.clone();
            }
        }
        "thread.turn-diff-completed" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                let turn_id = payload["turnId"].as_str().unwrap_or_default();
                thread
                    .checkpoints
                    .retain(|entry| entry["turnId"] != turn_id);
                thread.checkpoints.push(payload.clone());
                thread
                    .checkpoints
                    .sort_by_key(|entry| entry["checkpointTurnCount"].as_u64().unwrap_or_default());
                let state = match payload["status"].as_str().unwrap_or_default() {
                    "ready" => "completed",
                    "error" => "error",
                    _ => "error",
                };
                thread.latest_turn = Some(LatestTurn {
                    turn_id: turn_id.to_string(),
                    state: state.to_string(),
                    requested_at: thread
                        .latest_turn
                        .as_ref()
                        .filter(|entry| entry.turn_id == turn_id)
                        .map(|entry| entry.requested_at.clone())
                        .unwrap_or_else(|| {
                            payload["completedAt"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string()
                        }),
                    started_at: Some(
                        thread
                            .latest_turn
                            .as_ref()
                            .filter(|entry| entry.turn_id == turn_id)
                            .and_then(|entry| entry.started_at.clone())
                            .unwrap_or_else(|| {
                                payload["completedAt"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string()
                            }),
                    ),
                    completed_at: payload["completedAt"].as_str().map(ToOwned::to_owned),
                    assistant_message_id: payload["assistantMessageId"]
                        .as_str()
                        .map(ToOwned::to_owned),
                });
                thread.updated_at = occurred_at.clone();
            }
        }
        "thread.reverted" => {
            if let Some(thread) = find_thread_mut(snapshot, &payload) {
                let turn_count = payload["turnCount"].as_u64().unwrap_or_default();
                thread.checkpoints.retain(|checkpoint| {
                    checkpoint["checkpointTurnCount"]
                        .as_u64()
                        .unwrap_or_default()
                        <= turn_count
                });
                thread.updated_at = occurred_at.clone();
            }
        }
        _ => {}
    }

    snapshot.snapshot_sequence = event["sequence"].as_u64().unwrap_or_default();
    snapshot.updated_at = occurred_at;
}

fn find_thread_mut<'a>(
    snapshot: &'a mut ReadModelState,
    payload: &Value,
) -> Option<&'a mut Thread> {
    let thread_id = payload["threadId"].as_str().unwrap_or_default();
    snapshot
        .threads
        .iter_mut()
        .find(|thread| thread.id == thread_id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::apply;
    use crate::model::{ReadModelState, Thread};
    use crate::util::now_iso;

    #[test]
    fn merges_assistant_streaming_deltas() {
        let now = now_iso();
        let mut snapshot = ReadModelState {
            snapshot_sequence: 0,
            projects: vec![],
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

        apply(
            &mut snapshot,
            &json!({
                "sequence": 1,
                "occurredAt": now,
                "type": "thread.message-sent",
                "payload": {
                    "threadId": "thread-1",
                    "messageId": "assistant-1",
                    "role": "assistant",
                    "text": "hello",
                    "turnId": "turn-1",
                    "streaming": true,
                    "createdAt": "2026-01-01T00:00:00.000Z",
                    "updatedAt": "2026-01-01T00:00:00.000Z",
                }
            }),
        );
        apply(
            &mut snapshot,
            &json!({
                "sequence": 2,
                "occurredAt": "2026-01-01T00:00:01.000Z",
                "type": "thread.message-sent",
                "payload": {
                    "threadId": "thread-1",
                    "messageId": "assistant-1",
                    "role": "assistant",
                    "text": " world",
                    "turnId": "turn-1",
                    "streaming": true,
                    "createdAt": "2026-01-01T00:00:01.000Z",
                    "updatedAt": "2026-01-01T00:00:01.000Z",
                }
            }),
        );

        assert_eq!(snapshot.threads[0].messages[0].text, "hello world");
        assert!(snapshot.threads[0].messages[0].streaming);
    }
}
