mod attachments;
mod claude_adapter;
mod codex_adapter;
mod config;
mod decider;
mod diff;
mod git;
mod keybindings;
mod model;
mod open;
mod orchestration;
mod persistence;
mod projector;
mod provider_adapter;
mod provider_command_reactor;
mod provider_health;
mod provider_runtime;
mod provider_runtime_ingestion;
mod runtime;
mod state;
mod terminal;
mod util;
mod workspace;
mod ws_server;

pub use config::ServerConfig;
pub use runtime::{start_server, ServerHandle};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot, Mutex};
    use tokio::time::{sleep, Duration};

    struct FakeAdapter {
        provider_name: &'static str,
        pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    }

    impl FakeAdapter {
        fn new(provider_name: &'static str) -> Self {
            Self {
                provider_name,
                pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    #[async_trait]
    impl provider_adapter::ProviderAdapter for FakeAdapter {
        fn provider_name(&self) -> &'static str {
            self.provider_name
        }

        async fn start_session(
            &self,
            session: &mut provider_adapter::ProviderSessionState,
            _input: &provider_adapter::StartSessionInput,
        ) -> anyhow::Result<()> {
            session.provider_name = self.provider_name.to_string();
            if session.provider_session_id.is_none() {
                session.provider_session_id = Some(uuid::Uuid::new_v4().to_string());
            }
            Ok(())
        }

        async fn send_turn(
            &self,
            session: provider_adapter::ProviderSessionState,
            input: provider_adapter::SendTurnInput,
            events: mpsc::UnboundedSender<provider_adapter::AdapterEvent>,
            mut kill: oneshot::Receiver<()>,
        ) -> anyhow::Result<provider_adapter::ProviderSessionState> {
            if let Some(session_id) = session.provider_session_id.clone() {
                let _ = events.send(provider_adapter::AdapterEvent::SessionId(session_id));
            }
            let _ = events.send(provider_adapter::AdapterEvent::Runtime(
                provider_runtime_ingestion::RuntimeEvent::SessionSet {
                    thread_id: input.thread_id.clone(),
                    status: "running".to_string(),
                    provider_name: self.provider_name.to_string(),
                    runtime_mode: input.runtime_mode.clone(),
                    active_turn_id: Some(input.turn_id.clone()),
                    last_error: None,
                    updated_at: util::now_iso(),
                },
            ));

            if input.prompt.contains("[approval]") {
                let request_id = uuid::Uuid::new_v4().to_string();
                let (tx, rx) = oneshot::channel();
                self.pending_approvals
                    .lock()
                    .await
                    .insert(request_id.clone(), tx);
                let _ = events.send(provider_adapter::AdapterEvent::Runtime(
                    provider_runtime_ingestion::RuntimeEvent::Activity {
                        thread_id: input.thread_id.clone(),
                        activity: provider_runtime_ingestion::build_activity(
                            "approval",
                            "approval.requested",
                            "Approval requested before continuing the turn.",
                            json!({
                                "requestId": request_id,
                                "requestKind": "command",
                            }),
                            Some(&input.turn_id),
                        ),
                        created_at: util::now_iso(),
                    },
                ));
                tokio::select! {
                    _ = &mut kill => {
                        anyhow::bail!("turn interrupted");
                    }
                    decision = rx => {
                        let decision = decision.unwrap_or_else(|_| "deny".to_string());
                        let _ = events.send(provider_adapter::AdapterEvent::Runtime(
                            provider_runtime_ingestion::RuntimeEvent::Activity {
                                thread_id: input.thread_id.clone(),
                                activity: provider_runtime_ingestion::build_activity(
                                    "approval",
                                    "approval.resolved",
                                    "Approval resolved.",
                                    json!({
                                        "requestId": request_id,
                                        "decision": decision,
                                    }),
                                    Some(&input.turn_id),
                                ),
                                created_at: util::now_iso(),
                            },
                        ));
                    }
                }
            }

            let _ = events.send(provider_adapter::AdapterEvent::Runtime(
                provider_runtime_ingestion::RuntimeEvent::AssistantDelta {
                    thread_id: input.thread_id.clone(),
                    turn_id: input.turn_id.clone(),
                    message_id: input.assistant_message_id.clone(),
                    delta: format!("{} says: {}", self.provider_name, input.prompt),
                    created_at: util::now_iso(),
                },
            ));
            let _ = events.send(provider_adapter::AdapterEvent::Runtime(
                provider_runtime_ingestion::RuntimeEvent::AssistantComplete {
                    thread_id: input.thread_id,
                    turn_id: input.turn_id,
                    message_id: input.assistant_message_id,
                    created_at: util::now_iso(),
                },
            ));
            Ok(session)
        }

        async fn respond_to_approval(
            &self,
            _session: &provider_adapter::ProviderSessionState,
            _thread_id: &str,
            request_id: &str,
            decision: &str,
        ) -> anyhow::Result<()> {
            let sender = self.pending_approvals.lock().await.remove(request_id);
            let Some(sender) = sender else {
                anyhow::bail!("Unknown pending approval request.")
            };
            let _ = sender.send(decision.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn server_boots_with_empty_snapshot() {
        let server = start_server(test_config("Test")).await.unwrap();
        assert!(server.ws_url().starts_with("ws://127.0.0.1:"));
        server.shutdown().await;
    }

    #[test]
    fn rejects_parent_directory_write_paths() {
        assert!(util::ensure_relative_path("../escape.txt").is_err());
        assert!(util::ensure_relative_path("safe/file.txt").is_ok());
    }

    fn test_runtime_service() -> provider_runtime::ProviderRuntimeService {
        provider_runtime::ProviderRuntimeService::new_with_adapters(vec![
            Arc::new(FakeAdapter::new("codex")) as Arc<dyn provider_adapter::ProviderAdapter>,
            Arc::new(FakeAdapter::new("claudeAgent")) as Arc<dyn provider_adapter::ProviderAdapter>,
        ])
    }

    fn test_config(app_name: &str) -> ServerConfig {
        let root = std::env::temp_dir()
            .join("codex-wrapper-server-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let cwd = root.join("workspace");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        ServerConfig {
            app_name: app_name.to_string(),
            mode: config::RuntimeMode::Desktop,
            cwd: PathBuf::from(&cwd),
            state_dir,
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: None,
            dev_url: None,
            static_dir: None,
            no_browser: true,
            auto_bootstrap_project_from_cwd: false,
            log_websocket_events: false,
        }
    }

    async fn seed_project_and_thread(state: Arc<state::AppState>) {
        let created_at = util::now_iso();
        orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "project.create",
                "commandId": "cmd-project",
                "projectId": "project-1",
                "title": "Project",
                "workspaceRoot": state.cwd_string(),
                "defaultModel": "gpt-5-codex",
                "createdAt": created_at,
            }),
        )
        .await
        .unwrap();
        orchestration::handle_dispatch_command(
            state,
            &json!({
                "type": "thread.create",
                "commandId": "cmd-thread",
                "threadId": "thread-1",
                "projectId": "project-1",
                "title": "Thread",
                "model": "gpt-5-codex",
                "runtimeMode": "full-access",
                "interactionMode": "default",
                "branch": null,
                "worktreePath": null,
                "createdAt": util::now_iso(),
            }),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn thread_turn_start_runs_through_runtime_pipeline() {
        let state = Arc::new(state::AppState::new_with_provider_runtime(
            test_config("Test"),
            test_runtime_service(),
        )
        .await
        .unwrap());
        seed_project_and_thread(state.clone()).await;

        orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "thread.turn.start",
                "commandId": "cmd-turn",
                "threadId": "thread-1",
                "message": {
                    "messageId": "msg-user-1",
                    "role": "user",
                    "text": "hello runtime",
                    "attachments": [],
                },
                "provider": "codex",
                "createdAt": util::now_iso(),
            }),
        )
        .await
        .unwrap();

        let thread = loop {
            let snapshot = state.snapshot.lock().await.clone();
            let thread = snapshot
                .threads
                .iter()
                .find(|thread| thread.id == "thread-1")
                .unwrap()
                .clone();
            if thread
                .session
                .as_ref()
                .map(|session| session.status.as_str())
                == Some("ready")
            {
                break thread;
            }
            sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].role, "user");
        assert_eq!(thread.messages[1].role, "assistant");
        assert!(!thread.messages[1].text.is_empty());
        assert_eq!(
            thread
                .session
                .as_ref()
                .map(|session| session.status.as_str()),
            Some("ready")
        );
        assert_eq!(
            thread.latest_turn.as_ref().map(|turn| turn.state.as_str()),
            Some("completed")
        );
        assert_eq!(thread.checkpoints.len(), 1);
    }

    #[tokio::test]
    async fn approval_response_unblocks_waiting_turn() {
        let state = Arc::new(state::AppState::new_with_provider_runtime(
            test_config("Test"),
            test_runtime_service(),
        )
        .await
        .unwrap());
        seed_project_and_thread(state.clone()).await;

        orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "thread.turn.start",
                "commandId": "cmd-turn-approval",
                "threadId": "thread-1",
                "message": {
                    "messageId": "msg-user-approval",
                    "role": "user",
                    "text": "please continue [approval]",
                    "attachments": [],
                },
                "provider": "claudeAgent",
                "createdAt": util::now_iso(),
            }),
        )
        .await
        .unwrap();

        sleep(Duration::from_millis(100)).await;

        let pending_request_id = {
            let snapshot = state.snapshot.lock().await.clone();
            let thread = snapshot
                .threads
                .iter()
                .find(|thread| thread.id == "thread-1")
                .unwrap();
            assert_eq!(
                thread
                    .session
                    .as_ref()
                    .map(|session| session.status.as_str()),
                Some("running")
            );
            thread
                .activities
                .iter()
                .find(|activity| activity.kind == "approval.requested")
                .and_then(|activity| activity.payload["requestId"].as_str())
                .unwrap()
                .to_string()
        };

        orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "thread.approval.respond",
                "commandId": "cmd-approval-respond",
                "threadId": "thread-1",
                "requestId": pending_request_id,
                "decision": "approve_once",
                "createdAt": util::now_iso(),
            }),
        )
        .await
        .unwrap();

        let thread = loop {
            let snapshot = state.snapshot.lock().await.clone();
            let thread = snapshot
                .threads
                .iter()
                .find(|thread| thread.id == "thread-1")
                .unwrap()
                .clone();
            if thread
                .session
                .as_ref()
                .map(|session| session.status.as_str())
                == Some("ready")
            {
                break thread;
            }
            sleep(Duration::from_millis(50)).await;
        };
        assert!(thread
            .activities
            .iter()
            .any(|activity| activity.kind == "approval.resolved"));
        assert_eq!(
            thread
                .session
                .as_ref()
                .map(|session| session.status.as_str()),
            Some("ready")
        );
        assert!(thread
            .messages
            .iter()
            .any(|message| message.role == "assistant" && !message.text.is_empty()));
    }

    #[tokio::test]
    async fn persisted_events_reload_and_command_receipts_are_idempotent() {
        let config = test_config("Persistence");
        let state = Arc::new(
            state::AppState::new_with_provider_runtime(config.clone(), test_runtime_service())
                .await
                .unwrap(),
        );
        seed_project_and_thread(state.clone()).await;

        let first_response = orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "thread.meta.update",
                "commandId": "cmd-meta-update",
                "threadId": "thread-1",
                "title": "Renamed thread",
            }),
        )
        .await
        .unwrap();
        let first_sequence = first_response["sequence"].as_u64().unwrap_or_default();
        let first_event_count = state.events.lock().await.len();
        drop(state);

        let reloaded = Arc::new(
            state::AppState::new_with_provider_runtime(config, test_runtime_service())
                .await
                .unwrap(),
        );
        let snapshot = reloaded.snapshot.lock().await.clone();
        let thread = snapshot
            .threads
            .iter()
            .find(|thread| thread.id == "thread-1")
            .unwrap();
        assert_eq!(thread.title, "Renamed thread");
        assert_eq!(reloaded.events.lock().await.len(), first_event_count);

        let repeated_response = orchestration::handle_dispatch_command(
            reloaded.clone(),
            &json!({
                "type": "thread.meta.update",
                "commandId": "cmd-meta-update",
                "threadId": "thread-1",
                "title": "Renamed thread again",
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            repeated_response["sequence"].as_u64().unwrap_or_default(),
            first_sequence
        );
        assert_eq!(reloaded.events.lock().await.len(), first_event_count);
    }
}
