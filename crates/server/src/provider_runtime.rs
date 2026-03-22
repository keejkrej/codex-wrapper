use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::claude_adapter::ClaudeCliAdapter;
use crate::codex_adapter::CodexCliAdapter;
use crate::diff::summarize_thread_changes;
use crate::model::ChatAttachment;
use crate::persistence::ProviderSessionBinding;
use crate::provider_adapter::{
    AdapterEvent, ProviderAdapter, ProviderAdapterRegistry, ProviderSessionState, SendTurnInput,
    StartSessionInput,
};
use crate::provider_runtime_ingestion::{build_activity, ingest, RuntimeEvent};
use crate::state::AppState;
use crate::util::now_iso;

#[derive(Clone)]
pub(crate) struct ProviderRuntimeService {
    sessions: Arc<Mutex<HashMap<String, RuntimeSession>>>,
    adapters: Arc<ProviderAdapterRegistry>,
}

struct RuntimeSession {
    provider_name: String,
    runtime_mode: String,
    provider_session_id: Option<String>,
    current_turn: Option<RuntimeTurnHandle>,
}

struct RuntimeTurnHandle {
    turn_id: String,
    task: JoinHandle<()>,
    kill_tx: Option<oneshot::Sender<()>>,
}

pub(crate) struct StartTurnInput {
    pub thread_id: String,
    pub provider_name: String,
    pub runtime_mode: String,
    pub interaction_mode: String,
    pub prompt: String,
    pub cwd: String,
    pub state_dir: String,
    pub model: Option<String>,
    pub model_options: Option<Value>,
    pub provider_options: Option<Value>,
    pub assistant_delivery_mode: Option<String>,
    pub attachments: Vec<ChatAttachment>,
    pub created_at: String,
}

impl ProviderRuntimeService {
    pub(crate) fn new() -> Self {
        Self::new_with_adapters(vec![
            Arc::new(CodexCliAdapter) as Arc<dyn ProviderAdapter>,
            Arc::new(ClaudeCliAdapter) as Arc<dyn ProviderAdapter>,
        ])
    }

    pub(crate) fn new_with_adapters(adapters: Vec<Arc<dyn ProviderAdapter>>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            adapters: Arc::new(ProviderAdapterRegistry::new(adapters)),
        }
    }

    pub(crate) async fn restore_persisted_bindings(&self, bindings: Vec<ProviderSessionBinding>) {
        let mut sessions = self.sessions.lock().await;
        for binding in bindings {
            sessions.insert(
                binding.thread_id,
                RuntimeSession {
                    provider_name: binding.provider_name,
                    runtime_mode: binding.runtime_mode,
                    provider_session_id: binding.provider_session_id,
                    current_turn: None,
                },
            );
        }
    }

    pub(crate) async fn start_turn(&self, state: Arc<AppState>, input: StartTurnInput) {
        self.abort_existing_turn(&input.thread_id).await;

        let turn_id = uuid::Uuid::new_v4().to_string();
        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let adapter = match self.adapters.get(&input.provider_name) {
            Some(adapter) => adapter,
            None => {
                ingest(
                    &state,
                    RuntimeEvent::Activity {
                        thread_id: input.thread_id.clone(),
                        activity: build_activity(
                            "error",
                            "provider.runtime.failed",
                            "No provider adapter is available.",
                            json!({ "provider": input.provider_name }),
                            None,
                        ),
                        created_at: now_iso(),
                    },
                )
                .await;
                return;
            }
        };

        let mut session_state = {
            let mut sessions = self.sessions.lock().await;
            let session =
                sessions
                    .entry(input.thread_id.clone())
                    .or_insert_with(|| RuntimeSession {
                        provider_name: input.provider_name.clone(),
                        runtime_mode: input.runtime_mode.clone(),
                        provider_session_id: None,
                        current_turn: None,
                    });
            session.provider_name = input.provider_name.clone();
            session.runtime_mode = input.runtime_mode.clone();
            ProviderSessionState {
                provider_name: session.provider_name.clone(),
                runtime_mode: session.runtime_mode.clone(),
                provider_session_id: session.provider_session_id.clone(),
            }
        };

        if let Err(error) = adapter
            .start_session(
                &mut session_state,
                &StartSessionInput {
                    thread_id: input.thread_id.clone(),
                    cwd: input.cwd.clone(),
                    model: input.model.clone(),
                    runtime_mode: input.runtime_mode.clone(),
                },
            )
            .await
        {
            self.emit_runtime_error(
                state.clone(),
                &input.thread_id,
                &input.provider_name,
                &input.runtime_mode,
                error.to_string(),
            )
            .await;
            return;
        }

        {
            let mut sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&input.thread_id) {
                session.provider_session_id = session_state.provider_session_id.clone();
            }
        }
        self.persist_binding(
            &state,
            &input.thread_id,
            "starting",
            Some(turn_id.clone()),
            None,
        )
        .await;

        ingest(
            &state,
            RuntimeEvent::SessionSet {
                thread_id: input.thread_id.clone(),
                status: "starting".to_string(),
                provider_name: input.provider_name.clone(),
                runtime_mode: input.runtime_mode.clone(),
                active_turn_id: Some(turn_id.clone()),
                last_error: None,
                updated_at: input.created_at.clone(),
            },
        )
        .await;

        let service = self.clone();
        let thread_id = input.thread_id.clone();
        let provider_name = input.provider_name.clone();
        let runtime_mode = input.runtime_mode.clone();
        let turn_id_for_task = turn_id.clone();
        let assistant_message_id_for_task = assistant_message_id.clone();
        let (kill_tx, kill_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AdapterEvent>();
            let adapter_thread_id = thread_id.clone();
            let adapter_provider_name = provider_name.clone();
            let adapter_runtime_mode = runtime_mode.clone();
            let state_for_send = state.clone();
            let service_for_events = service.clone();
            let turn_id_for_send = turn_id_for_task.clone();
            let assistant_message_id_for_send = assistant_message_id_for_task.clone();
            let send_handle = tokio::spawn(async move {
                adapter
                    .send_turn(
                        session_state,
                        SendTurnInput {
                            thread_id: adapter_thread_id.clone(),
                            turn_id: turn_id_for_send,
                            assistant_message_id: assistant_message_id_for_send,
                            cwd: input.cwd.clone(),
                            state_dir: input.state_dir.clone(),
                            prompt: input.prompt.clone(),
                            model: input.model.clone(),
                            model_options: input.model_options.clone(),
                            provider_options: input.provider_options.clone(),
                            runtime_mode: input.runtime_mode.clone(),
                            interaction_mode: input.interaction_mode.clone(),
                            assistant_delivery_mode: input.assistant_delivery_mode.clone(),
                            attachments: input.attachments.clone(),
                            created_at: input.created_at.clone(),
                        },
                        event_tx,
                        kill_rx,
                    )
                    .await
            });

            while let Some(event) = event_rx.recv().await {
                match event {
                    AdapterEvent::SessionId(session_id) => {
                        service_for_events
                            .update_provider_session_id(&state, &thread_id, Some(session_id))
                            .await;
                    }
                    AdapterEvent::Runtime(runtime_event) => {
                        ingest(&state, runtime_event).await;
                    }
                }
            }

            match send_handle.await {
                Ok(Ok(updated_session)) => {
                    service
                        .update_provider_session_id(
                            &state_for_send,
                            &thread_id,
                            updated_session.provider_session_id.clone(),
                        )
                        .await;
                    let diff_summary = summarize_thread_changes(&state_for_send, &thread_id).await;
                    let checkpoint_turn_count = {
                        let snapshot = state_for_send.snapshot.lock().await;
                        snapshot
                            .threads
                            .iter()
                            .find(|thread| thread.id == thread_id)
                            .map(|thread| thread.checkpoints.len() as u64 + 1)
                            .unwrap_or(1)
                    };
                    ingest(
                        &state_for_send,
                        RuntimeEvent::TurnDiffComplete {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id_for_task.clone(),
                            checkpoint_turn_count,
                            checkpoint_ref: if diff_summary.checkpoint_ref.is_empty() {
                                "HEAD".to_string()
                            } else {
                                diff_summary.checkpoint_ref
                            },
                            status: "ready".to_string(),
                            files: diff_summary.files,
                            assistant_message_id: Some(assistant_message_id_for_task.clone()),
                            completed_at: now_iso(),
                        },
                    )
                    .await;
                    ingest(
                        &state_for_send,
                        RuntimeEvent::SessionSet {
                            thread_id: thread_id.clone(),
                            status: "ready".to_string(),
                            provider_name: adapter_provider_name,
                            runtime_mode: adapter_runtime_mode,
                            active_turn_id: None,
                            last_error: None,
                            updated_at: now_iso(),
                        },
                    )
                    .await;
                    service
                        .persist_binding(&state_for_send, &thread_id, "ready", None, None)
                        .await;
                }
                Ok(Err(error)) => {
                    service
                        .emit_runtime_error(
                            state_for_send.clone(),
                            &thread_id,
                            &adapter_provider_name,
                            &adapter_runtime_mode,
                            error.to_string(),
                        )
                        .await;
                }
                Err(error) => {
                    service
                        .emit_runtime_error(
                            state_for_send.clone(),
                            &thread_id,
                            &adapter_provider_name,
                            &adapter_runtime_mode,
                            error.to_string(),
                        )
                        .await;
                }
            }
            service.finish_turn(&thread_id).await;
        });

        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&input.thread_id) {
            session.current_turn = Some(RuntimeTurnHandle {
                turn_id,
                task,
                kill_tx: Some(kill_tx),
            });
        }
    }

    pub(crate) async fn interrupt_turn(
        &self,
        state: Arc<AppState>,
        thread_id: &str,
        turn_id: Option<&str>,
    ) {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.get_mut(thread_id) else {
            return;
        };
        if let Some(current_turn) = session.current_turn.take() {
            if turn_id.is_none() || turn_id == Some(current_turn.turn_id.as_str()) {
                if let Some(adapter) = self.adapters.get(&session.provider_name) {
                    let session_state = ProviderSessionState {
                        provider_name: session.provider_name.clone(),
                        runtime_mode: session.runtime_mode.clone(),
                        provider_session_id: session.provider_session_id.clone(),
                    };
                    let _ = adapter
                        .interrupt_turn(&session_state, thread_id, turn_id)
                        .await;
                }
                if let Some(kill_tx) = current_turn.kill_tx {
                    let _ = kill_tx.send(());
                }
                current_turn.task.abort();
                let provider_name = session.provider_name.clone();
                let runtime_mode = session.runtime_mode.clone();
                drop(sessions);
                ingest(
                    &state,
                    RuntimeEvent::Activity {
                        thread_id: thread_id.to_string(),
                        activity: build_activity(
                            "info",
                            "turn.interrupted",
                            "Interrupted the active turn.",
                            json!({ "turnId": current_turn.turn_id }),
                            turn_id,
                        ),
                        created_at: now_iso(),
                    },
                )
                .await;
                ingest(
                    &state,
                    RuntimeEvent::SessionSet {
                        thread_id: thread_id.to_string(),
                        status: "interrupted".to_string(),
                        provider_name,
                        runtime_mode,
                        active_turn_id: None,
                        last_error: None,
                        updated_at: now_iso(),
                    },
                )
                .await;
                self
                    .persist_binding(&state, thread_id, "interrupted", None, None)
                    .await;
            }
        }
    }

    pub(crate) async fn stop_session(&self, state: Arc<AppState>, thread_id: &str) {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.get_mut(thread_id) else {
            return;
        };
        if let Some(current_turn) = session.current_turn.take() {
            if let Some(kill_tx) = current_turn.kill_tx {
                let _ = kill_tx.send(());
            }
            current_turn.task.abort();
        }
        if let Some(adapter) = self.adapters.get(&session.provider_name) {
            let session_state = ProviderSessionState {
                provider_name: session.provider_name.clone(),
                runtime_mode: session.runtime_mode.clone(),
                provider_session_id: session.provider_session_id.clone(),
            };
            let _ = adapter.stop_session(&session_state, thread_id).await;
        }
        let provider_name = session.provider_name.clone();
        let runtime_mode = session.runtime_mode.clone();
        drop(sessions);
        let _ = state.delete_provider_session_binding(thread_id).await;
        ingest(
            &state,
            RuntimeEvent::SessionSet {
                thread_id: thread_id.to_string(),
                status: "stopped".to_string(),
                provider_name,
                runtime_mode,
                active_turn_id: None,
                last_error: None,
                updated_at: now_iso(),
            },
        )
        .await;
    }

    pub(crate) async fn respond_to_approval(
        &self,
        state: Arc<AppState>,
        thread_id: &str,
        request_id: &str,
        decision: &str,
    ) {
        let session_state = self.session_state(thread_id).await;
        let Some(session_state) = session_state else {
            return;
        };
        let Some(adapter) = self.adapters.get(&session_state.provider_name) else {
            return;
        };
        if let Err(error) = adapter
            .respond_to_approval(&session_state, thread_id, request_id, decision)
            .await
        {
            ingest(
                &state,
                RuntimeEvent::Activity {
                    thread_id: thread_id.to_string(),
                    activity: build_activity(
                        "error",
                        "provider.approval.respond.failed",
                        "Could not resolve approval response.",
                        json!({
                            "requestId": request_id,
                            "detail": error.to_string(),
                        }),
                        None,
                    ),
                    created_at: now_iso(),
                },
            )
            .await;
        }
    }

    pub(crate) async fn respond_to_user_input(
        &self,
        state: Arc<AppState>,
        thread_id: &str,
        request_id: &str,
        answers: Value,
    ) {
        let session_state = self.session_state(thread_id).await;
        let Some(session_state) = session_state else {
            return;
        };
        let Some(adapter) = self.adapters.get(&session_state.provider_name) else {
            return;
        };
        if let Err(error) = adapter
            .respond_to_user_input(&session_state, thread_id, request_id, &answers)
            .await
        {
            ingest(
                &state,
                RuntimeEvent::Activity {
                    thread_id: thread_id.to_string(),
                    activity: build_activity(
                        "error",
                        "provider.user-input.respond.failed",
                        "Could not resolve user-input response.",
                        json!({
                            "requestId": request_id,
                            "detail": error.to_string(),
                        }),
                        None,
                    ),
                    created_at: now_iso(),
                },
            )
            .await;
        }
    }

    async fn abort_existing_turn(&self, thread_id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(thread_id) {
            if let Some(current_turn) = session.current_turn.take() {
                if let Some(kill_tx) = current_turn.kill_tx {
                    let _ = kill_tx.send(());
                }
                current_turn.task.abort();
            }
        }
    }

    async fn update_provider_session_id(
        &self,
        state: &AppState,
        thread_id: &str,
        session_id: Option<String>,
    ) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(thread_id) {
            session.provider_session_id = session_id;
        }
        drop(sessions);
        self.persist_binding(state, thread_id, "running", self.active_turn_id(thread_id).await, None)
            .await;
    }

    async fn finish_turn(&self, thread_id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(thread_id) {
            session.current_turn = None;
        }
    }

    async fn session_state(&self, thread_id: &str) -> Option<ProviderSessionState> {
        let sessions = self.sessions.lock().await;
        sessions.get(thread_id).map(|session| ProviderSessionState {
            provider_name: session.provider_name.clone(),
            runtime_mode: session.runtime_mode.clone(),
            provider_session_id: session.provider_session_id.clone(),
        })
    }

    async fn emit_runtime_error(
        &self,
        state: Arc<AppState>,
        thread_id: &str,
        provider_name: &str,
        runtime_mode: &str,
        detail: String,
    ) {
        ingest(
            &state,
            RuntimeEvent::Activity {
                thread_id: thread_id.to_string(),
                activity: build_activity(
                    "error",
                    "provider.runtime.failed",
                    "Provider runtime failed.",
                    json!({ "detail": detail.clone() }),
                    None,
                ),
                created_at: now_iso(),
            },
        )
        .await;
        ingest(
            &state,
            RuntimeEvent::SessionSet {
                thread_id: thread_id.to_string(),
                status: "error".to_string(),
                provider_name: provider_name.to_string(),
                runtime_mode: runtime_mode.to_string(),
                active_turn_id: None,
                last_error: Some(detail.clone()),
                updated_at: now_iso(),
            },
        )
        .await;
        self.persist_binding(
            &state,
            thread_id,
            "error",
            None,
            Some(detail.clone()),
        )
        .await;
        self.finish_turn(thread_id).await;
    }

    async fn active_turn_id(&self, thread_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(thread_id)
            .and_then(|session| session.current_turn.as_ref().map(|turn| turn.turn_id.clone()))
    }

    async fn persist_binding(
        &self,
        state: &AppState,
        thread_id: &str,
        status: &str,
        active_turn_id: Option<String>,
        last_error: Option<String>,
    ) {
        let sessions = self.sessions.lock().await;
        let Some(session) = sessions.get(thread_id) else {
            return;
        };
        let _ = state
            .upsert_provider_session_binding(ProviderSessionBinding {
                thread_id: thread_id.to_string(),
                provider_name: session.provider_name.clone(),
                runtime_mode: session.runtime_mode.clone(),
                provider_session_id: session.provider_session_id.clone(),
                status: status.to_string(),
                active_turn_id,
                last_error,
                updated_at: now_iso(),
            })
            .await;
    }
}
