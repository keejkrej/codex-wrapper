use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::config::ServerConfig;
use crate::state::AppState;
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

    let state = Arc::new(AppState::new(config.clone()));
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
    let ws_url = format!("ws://{}:{}/ws", host, local_addr.port());
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
