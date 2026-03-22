use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::decider::{self, DecidedEvent};
use crate::keybindings::{keybindings_config_path, load_resolved_keybindings};
use crate::model::{ReadModelState, TerminalSession};
use crate::projector;
use crate::provider_health::provider_statuses;
use crate::provider_runtime::ProviderRuntimeService;
use crate::util::now_iso;
use crate::ws_server::{
    WS_CHANNEL_ORCHESTRATION_DOMAIN_EVENT, WS_CHANNEL_SERVER_CONFIG_UPDATED,
    WS_CHANNEL_TERMINAL_EVENT,
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub config: ServerConfig,
    pub snapshot: Arc<Mutex<ReadModelState>>,
    pub events: Arc<Mutex<Vec<Value>>>,
    pub sequence: Arc<AtomicU64>,
    pub pushes: broadcast::Sender<Value>,
    pub terminals: Arc<Mutex<HashMap<String, TerminalSession>>>,
    pub provider_runtime: ProviderRuntimeService,
}

impl AppState {
    pub(crate) fn new(config: ServerConfig) -> Self {
        Self::new_with_provider_runtime(config, ProviderRuntimeService::new())
    }

    pub(crate) fn new_with_provider_runtime(
        config: ServerConfig,
        provider_runtime: ProviderRuntimeService,
    ) -> Self {
        let (pushes, _) = broadcast::channel(256);
        Self {
            config,
            snapshot: Arc::new(Mutex::new(ReadModelState {
                snapshot_sequence: 0,
                projects: Vec::new(),
                threads: Vec::new(),
                updated_at: now_iso(),
            })),
            events: Arc::new(Mutex::new(Vec::new())),
            sequence: Arc::new(AtomicU64::new(0)),
            pushes,
            terminals: Arc::new(Mutex::new(HashMap::new())),
            provider_runtime,
        }
    }

    pub(crate) fn cwd_string(&self) -> String {
        crate::util::normalize_path(&self.config.cwd)
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) async fn snapshot_value(&self) -> Result<Value> {
        Ok(serde_json::to_value(self.snapshot.lock().await.clone())?)
    }

    pub(crate) async fn replay_events(&self, from_sequence_exclusive: u64) -> Result<Value> {
        let events = self.events.lock().await;
        Ok(Value::Array(
            events
                .iter()
                .filter(|event| {
                    event["sequence"].as_u64().unwrap_or_default() > from_sequence_exclusive
                })
                .cloned()
                .collect(),
        ))
    }

    pub(crate) async fn emit_push(&self, channel: &str, data: Value) -> Result<()> {
        let push = json!({
            "type": "push",
            "sequence": self.next_sequence(),
            "channel": channel,
            "data": data,
        });
        let _ = self.pushes.send(push);
        Ok(())
    }

    pub(crate) async fn dispatch_internal_command(&self, command: &Value) -> Result<Vec<Value>> {
        let snapshot = self.snapshot.lock().await.clone();
        let decided_events = decider::decide(&snapshot, &self.config, command).await?;
        self.append_decided_events(decided_events).await
    }

    pub(crate) async fn append_decided_events(
        &self,
        decided_events: Vec<DecidedEvent>,
    ) -> Result<Vec<Value>> {
        let mut appended = Vec::with_capacity(decided_events.len());
        for decided_event in decided_events {
            let sequence = self.next_sequence();
            let event = json!({
                "sequence": sequence,
                "eventId": Uuid::new_v4().to_string(),
                "aggregateKind": decided_event.aggregate_kind,
                "aggregateId": decided_event.aggregate_id,
                "occurredAt": decided_event.occurred_at,
                "commandId": decided_event.command_id,
                "causationEventId": decided_event.causation_event_id,
                "correlationId": decided_event.correlation_id,
                "metadata": decided_event.metadata,
                "type": decided_event.event_type,
                "payload": decided_event.payload,
            });
            self.events.lock().await.push(event.clone());
            {
                let mut snapshot = self.snapshot.lock().await;
                projector::apply(&mut snapshot, &event);
            }
            self.emit_push(WS_CHANNEL_ORCHESTRATION_DOMAIN_EVENT, event.clone())
                .await?;
            appended.push(event);
        }
        Ok(appended)
    }

    pub(crate) async fn provider_statuses(&self) -> Value {
        provider_statuses().await
    }

    pub(crate) async fn config_updated_payload(&self) -> Value {
        let (_, issues) = load_resolved_keybindings(&self.config.cwd).unwrap_or_else(|error| {
            (
                Vec::new(),
                vec![json!({
                    "kind": "keybindings.malformed-config",
                    "message": error.to_string()
                })],
            )
        });
        json!({
            "issues": issues,
            "providers": self.provider_statuses().await,
        })
    }

    pub(crate) async fn emit_config_updated(&self) -> Result<()> {
        let payload = self.config_updated_payload().await;
        self.emit_push(WS_CHANNEL_SERVER_CONFIG_UPDATED, payload)
            .await
    }

    pub(crate) async fn emit_terminal_event(&self, data: Value) -> Result<()> {
        self.emit_push(WS_CHANNEL_TERMINAL_EVENT, data).await
    }

    pub(crate) async fn server_config_value(&self) -> Value {
        let (keybindings, issues) =
            load_resolved_keybindings(&self.config.cwd).unwrap_or_else(|error| {
                (
                    Vec::new(),
                    vec![json!({
                        "kind": "keybindings.malformed-config",
                        "message": error.to_string()
                    })],
                )
            });
        json!({
            "cwd": self.cwd_string(),
            "keybindingsConfigPath": keybindings_config_path(&self.config.cwd),
            "keybindings": keybindings,
            "issues": issues,
            "providers": self.provider_statuses().await,
            "availableEditors": ["file-manager", "cursor", "vscode", "zed", "antigravity"]
        })
    }
}
