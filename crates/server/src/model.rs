use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::ChildStdin;
use tokio::sync::{oneshot, Mutex};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScript {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: String,
    pub run_on_worktree_create: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
    pub default_model: Option<String>,
    pub scripts: Vec<ProjectScript>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub attachments: Vec<ChatAttachment>,
    pub turn_id: Option<String>,
    pub streaming: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadActivity {
    pub id: String,
    pub tone: String,
    pub kind: String,
    pub summary: String,
    pub payload: Value,
    pub turn_id: Option<String>,
    pub sequence: Option<u64>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestTurn {
    pub turn_id: String,
    pub state: String,
    pub requested_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub assistant_message_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSession {
    pub thread_id: String,
    pub status: String,
    pub provider_name: Option<String>,
    pub runtime_mode: String,
    pub active_turn_id: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub model: String,
    pub runtime_mode: String,
    pub interaction_mode: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub latest_turn: Option<LatestTurn>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub messages: Vec<ThreadMessage>,
    pub proposed_plans: Vec<Value>,
    pub activities: Vec<ThreadActivity>,
    pub checkpoints: Vec<Value>,
    pub session: Option<ThreadSession>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadModelState {
    pub snapshot_sequence: u64,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub updated_at: String,
}

pub struct TerminalSession {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub history: Arc<Mutex<String>>,
    pub stdin: Arc<Mutex<ChildStdin>>,
    pub kill_tx: Option<oneshot::Sender<()>>,
    pub env: Option<HashMap<String, String>>,
    pub cols: i64,
    pub rows: i64,
}
