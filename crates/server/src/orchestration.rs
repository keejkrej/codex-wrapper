use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::decider;
use crate::provider_command_reactor::react_to_events;
use crate::state::AppState;

pub(crate) async fn handle_dispatch_command(
    state: Arc<AppState>,
    command: &Value,
) -> Result<Value> {
    let command_type = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Command type is required"))?;

    let snapshot = state.snapshot.lock().await.clone();
    let decided_events = decider::decide(&snapshot, &state.config, command).await?;
    let appended_events = state.append_decided_events(decided_events).await?;

    react_to_events(
        state.clone(),
        state.provider_runtime.clone(),
        &appended_events,
    )
    .await;

    let sequence = appended_events
        .last()
        .and_then(|event| event.get("sequence"))
        .and_then(Value::as_u64)
        .unwrap_or_default();

    if command_type == "thread.checkpoint.revert" {
        return Ok(json!({ "sequence": sequence }));
    }

    Ok(json!({ "sequence": sequence }))
}
