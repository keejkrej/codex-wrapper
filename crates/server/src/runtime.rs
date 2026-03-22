use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::config::{RuntimeMode, ServerConfig};
use crate::orchestration;
use crate::state::AppState;
use crate::util::now_iso;
use crate::ws_server::router;

#[derive(Clone)]
pub struct ServerHandle {
    inner: Arc<ServerInner>,
}

impl ServerHandle {
    pub fn ws_url(&self) -> String {
        self.inner.ws_url.clone()
    }

    pub fn http_url(&self) -> String {
        self.inner.http_url.clone()
    }

    pub async fn shutdown(&self) {
        if let Some(shutdown_tx) = self.inner.shutdown_tx.lock().await.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

struct ServerInner {
    _state: Arc<AppState>,
    ws_url: String,
    http_url: String,
    _server_task: JoinHandle<()>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

pub async fn start_server(config: ServerConfig) -> Result<ServerHandle> {
    let cwd = config.cwd.canonicalize().unwrap_or(config.cwd.clone());
    let mut config = config;
    config.cwd = cwd;
    if config.mode == RuntimeMode::Desktop && config.auth_token.is_none() {
        config.auth_token = Some(Uuid::new_v4().to_string());
    }

    let state = Arc::new(AppState::new(config.clone()).await?);
    if config.auto_bootstrap_project_from_cwd {
        auto_bootstrap_project_from_cwd(state.clone()).await?;
    }
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;
    let local_addr = listener.local_addr()?;
    let host = if local_addr.ip().is_loopback() {
        "127.0.0.1".to_string()
    } else if local_addr.is_ipv6() {
        format!("[{}]", local_addr.ip())
    } else {
        local_addr.ip().to_string()
    };
    let http_url = format!("http://{}:{}", host, local_addr.port());
    let ws_url = match config.auth_token.as_deref() {
        Some(token) => format!("ws://{}:{}/ws?token={}", host, local_addr.port(), token),
        None => format!("ws://{}:{}/ws", host, local_addr.port()),
    };
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let app = router(state.clone());
    let server_task = tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        let _ = server.await;
    });

    Ok(ServerHandle {
        inner: Arc::new(ServerInner {
            _state: state,
            ws_url,
            http_url,
            _server_task: server_task,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        }),
    })
}

async fn auto_bootstrap_project_from_cwd(state: Arc<AppState>) -> Result<()> {
    let cwd = state.cwd_string();
    let snapshot = state.snapshot.lock().await.clone();
    let existing_project = snapshot
        .projects
        .iter()
        .find(|project| project.workspace_root == cwd && project.deleted_at.is_none())
        .cloned();
    let project_id = existing_project
        .as_ref()
        .map(|project| project.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let default_model = existing_project
        .as_ref()
        .and_then(|project| project.default_model.clone())
        .unwrap_or_else(|| "gpt-5-codex".to_string());

    if existing_project.is_none() {
        orchestration::handle_dispatch_command(
            state.clone(),
            &json!({
                "type": "project.create",
                "commandId": Uuid::new_v4().to_string(),
                "projectId": project_id,
                "title": state
                    .config
                    .cwd
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "project".to_string()),
                "workspaceRoot": cwd,
                "defaultModel": default_model,
                "createdAt": now_iso(),
            }),
        )
        .await?;
    }

    let snapshot = state.snapshot.lock().await.clone();
    let has_thread = snapshot
        .threads
        .iter()
        .any(|thread| thread.project_id == project_id && thread.deleted_at.is_none());
    if !has_thread {
        orchestration::handle_dispatch_command(
            state,
            &json!({
                "type": "thread.create",
                "commandId": Uuid::new_v4().to_string(),
                "threadId": Uuid::new_v4().to_string(),
                "projectId": project_id,
                "title": "New thread",
                "model": default_model,
                "runtimeMode": "full-access",
                "interactionMode": "default",
                "branch": null,
                "worktreePath": null,
                "createdAt": now_iso(),
            }),
        )
        .await?;
    }

    Ok(())
}
