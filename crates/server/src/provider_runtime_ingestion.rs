use serde_json::{json, Value};

use crate::state::AppState;
use crate::util::now_iso;

#[derive(Clone, Debug)]
pub(crate) enum RuntimeEvent {
    SessionSet {
        thread_id: String,
        status: String,
        provider_name: String,
        runtime_mode: String,
        active_turn_id: Option<String>,
        last_error: Option<String>,
        updated_at: String,
    },
    AssistantDelta {
        thread_id: String,
        turn_id: String,
        message_id: String,
        delta: String,
        created_at: String,
    },
    AssistantComplete {
        thread_id: String,
        turn_id: String,
        message_id: String,
        created_at: String,
    },
    Activity {
        thread_id: String,
        activity: Value,
        created_at: String,
    },
    TurnDiffComplete {
        thread_id: String,
        turn_id: String,
        checkpoint_turn_count: u64,
        checkpoint_ref: String,
        status: String,
        files: Vec<Value>,
        assistant_message_id: Option<String>,
        completed_at: String,
    },
}

pub(crate) async fn ingest(state: &AppState, runtime_event: RuntimeEvent) {
    let command = match runtime_event {
        RuntimeEvent::SessionSet {
            thread_id,
            status,
            provider_name,
            runtime_mode,
            active_turn_id,
            last_error,
            updated_at,
        } => json!({
            "type": "thread.session.set",
            "threadId": thread_id.clone(),
            "session": {
                "threadId": thread_id,
                "status": status,
                "providerName": provider_name,
                "runtimeMode": runtime_mode,
                "activeTurnId": active_turn_id,
                "lastError": last_error,
                "updatedAt": updated_at,
            },
            "createdAt": updated_at,
        }),
        RuntimeEvent::AssistantDelta {
            thread_id,
            turn_id,
            message_id,
            delta,
            created_at,
        } => json!({
            "type": "thread.message.assistant.delta",
            "threadId": thread_id,
            "turnId": turn_id,
            "messageId": message_id,
            "delta": delta,
            "createdAt": created_at,
        }),
        RuntimeEvent::AssistantComplete {
            thread_id,
            turn_id,
            message_id,
            created_at,
        } => json!({
            "type": "thread.message.assistant.complete",
            "threadId": thread_id,
            "turnId": turn_id,
            "messageId": message_id,
            "createdAt": created_at,
        }),
        RuntimeEvent::Activity {
            thread_id,
            activity,
            created_at,
        } => json!({
            "type": "thread.activity.append",
            "threadId": thread_id,
            "activity": activity,
            "createdAt": created_at,
        }),
        RuntimeEvent::TurnDiffComplete {
            thread_id,
            turn_id,
            checkpoint_turn_count,
            checkpoint_ref,
            status,
            files,
            assistant_message_id,
            completed_at,
        } => json!({
            "type": "thread.turn.diff.complete",
            "threadId": thread_id,
            "turnId": turn_id,
            "checkpointTurnCount": checkpoint_turn_count,
            "checkpointRef": checkpoint_ref,
            "status": status,
            "files": files,
            "assistantMessageId": assistant_message_id,
            "completedAt": completed_at,
        }),
    };

    let _ = state.dispatch_internal_command(&command).await;
}

pub(crate) fn build_activity(
    tone: &str,
    kind: &str,
    summary: &str,
    payload: Value,
    turn_id: Option<&str>,
) -> Value {
    json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "tone": tone,
        "kind": kind,
        "summary": summary,
        "payload": payload,
        "turnId": turn_id,
        "createdAt": now_iso(),
    })
}
