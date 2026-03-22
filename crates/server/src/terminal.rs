use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

use crate::state::AppState;
use crate::util::{now_iso, required_string_from_object};

fn shell_command() -> (String, Vec<String>) {
    if cfg!(target_os = "windows") {
        if let Ok(comspec) = std::env::var("COMSPEC") {
            return (comspec, Vec::new());
        }
        return ("cmd.exe".to_string(), Vec::new());
    }
    if let Ok(shell) = std::env::var("SHELL") {
        return (shell, Vec::new());
    }
    ("/bin/sh".to_string(), Vec::new())
}

pub(crate) async fn open_terminal(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<Value> {
    let thread_id = required_string_from_object(body, "threadId")?;
    let terminal_id = body
        .get("terminalId")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_string();
    let cwd = required_string_from_object(body, "cwd")?;
    let cols = body.get("cols").and_then(Value::as_i64).unwrap_or(120);
    let rows = body.get("rows").and_then(Value::as_i64).unwrap_or(30);
    let env = body.get("env").and_then(Value::as_object).map(|map| {
        map.iter()
            .filter_map(|(key, value)| value.as_str().map(|raw| (key.clone(), raw.to_string())))
            .collect::<HashMap<_, _>>()
    });
    let key = format!("{thread_id}:{terminal_id}");
    terminal_close_by_key(state.clone(), &key).await?;

    let (program, args) = shell_command();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env_map) = env.clone() {
        command.envs(env_map.clone());
    }
    let mut child = command.spawn().context("failed to spawn terminal shell")?;
    let pid = child.id().map(|value| value as i64);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to acquire terminal stdin"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let history = Arc::new(Mutex::new(String::new()));
    if let Some(stdout) = stdout {
        tokio::spawn(read_terminal_stream(
            stdout,
            history.clone(),
            state.clone(),
            thread_id.clone(),
            terminal_id.clone(),
        ));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(read_terminal_stream(
            stderr,
            history.clone(),
            state.clone(),
            thread_id.clone(),
            terminal_id.clone(),
        ));
    }
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    tokio::spawn(watch_terminal_process(
        state.clone(),
        key.clone(),
        thread_id.clone(),
        terminal_id.clone(),
        child,
        kill_rx,
    ));
    state.terminals.lock().await.insert(
        key,
        crate::model::TerminalSession {
            thread_id: thread_id.clone(),
            terminal_id: terminal_id.clone(),
            cwd: cwd.clone(),
            history: history.clone(),
            stdin: Arc::new(Mutex::new(stdin)),
            kill_tx: Some(kill_tx),
            env: env.clone(),
            cols,
            rows,
        },
    );
    let snapshot = json!({
        "threadId": thread_id,
        "terminalId": terminal_id,
        "cwd": cwd,
        "status": "running",
        "pid": pid,
        "history": "",
        "exitCode": Value::Null,
        "exitSignal": Value::Null,
        "updatedAt": now_iso()
    });
    state
        .emit_terminal_event(json!({
            "threadId": snapshot["threadId"].clone(),
            "terminalId": snapshot["terminalId"].clone(),
            "createdAt": now_iso(),
            "type": "started",
            "snapshot": snapshot
        }))
        .await?;
    Ok(snapshot)
}

pub(crate) async fn terminal_write(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<()> {
    let key = terminal_key(body);
    let data = required_string_from_object(body, "data")?;
    let sessions = state.terminals.lock().await;
    let session = sessions
        .get(&key)
        .ok_or_else(|| anyhow!("Terminal session not found"))?;
    let mut stdin = session.stdin.lock().await;
    stdin.write_all(data.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

pub(crate) async fn terminal_resize(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<()> {
    let key = terminal_key(body);
    let cols = body.get("cols").and_then(Value::as_i64).unwrap_or(120);
    let rows = body.get("rows").and_then(Value::as_i64).unwrap_or(30);
    let mut sessions = state.terminals.lock().await;
    if let Some(session) = sessions.get_mut(&key) {
        session.cols = cols;
        session.rows = rows;
    }
    Ok(())
}

pub(crate) async fn terminal_clear(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<()> {
    let key = terminal_key(body);
    let sessions = state.terminals.lock().await;
    let session = sessions
        .get(&key)
        .ok_or_else(|| anyhow!("Terminal session not found"))?;
    *session.history.lock().await = String::new();
    state
        .emit_terminal_event(json!({
            "threadId": session.thread_id,
            "terminalId": session.terminal_id,
            "createdAt": now_iso(),
            "type": "cleared"
        }))
        .await?;
    Ok(())
}

pub(crate) async fn restart_terminal(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<Value> {
    let key = terminal_key(body);
    let existing = {
        let sessions = state.terminals.lock().await;
        let session = sessions
            .get(&key)
            .ok_or_else(|| anyhow!("Terminal session not found"))?;
        (
            session.thread_id.clone(),
            session.terminal_id.clone(),
            session.cwd.clone(),
            session.env.clone(),
        )
    };
    terminal_close_by_key(state.clone(), &key).await?;
    let mut open_body = serde_json::Map::new();
    open_body.insert("threadId".to_string(), json!(existing.0));
    open_body.insert("terminalId".to_string(), json!(existing.1));
    open_body.insert("cwd".to_string(), json!(existing.2));
    open_body.insert(
        "cols".to_string(),
        body.get("cols").cloned().unwrap_or(json!(120)),
    );
    open_body.insert(
        "rows".to_string(),
        body.get("rows").cloned().unwrap_or(json!(30)),
    );
    if let Some(env) = existing.3 {
        open_body.insert("env".to_string(), json!(env));
    }
    let snapshot = open_terminal(state.clone(), &open_body).await?;
    state
        .emit_terminal_event(json!({
            "threadId": snapshot["threadId"].clone(),
            "terminalId": snapshot["terminalId"].clone(),
            "createdAt": now_iso(),
            "type": "restarted",
            "snapshot": snapshot
        }))
        .await?;
    Ok(snapshot)
}

pub(crate) async fn terminal_close(
    state: Arc<AppState>,
    body: &serde_json::Map<String, Value>,
) -> Result<()> {
    if let Some(terminal_id) = body.get("terminalId").and_then(Value::as_str) {
        let key = format!(
            "{}:{}",
            required_string_from_object(body, "threadId")?,
            terminal_id
        );
        terminal_close_by_key(state, &key).await?;
        return Ok(());
    }
    let thread_id = required_string_from_object(body, "threadId")?;
    let keys = {
        let sessions = state.terminals.lock().await;
        sessions
            .keys()
            .filter(|key| key.starts_with(&format!("{thread_id}:")))
            .cloned()
            .collect::<Vec<_>>()
    };
    for key in keys {
        terminal_close_by_key(state.clone(), &key).await?;
    }
    Ok(())
}

async fn terminal_close_by_key(state: Arc<AppState>, key: &str) -> Result<()> {
    let session = state.terminals.lock().await.remove(key);
    if let Some(mut session) = session {
        if let Some(kill_tx) = session.kill_tx.take() {
            let _ = kill_tx.send(());
        }
    }
    Ok(())
}

async fn read_terminal_stream<R: AsyncRead + Unpin>(
    mut stream: R,
    history: Arc<Mutex<String>>,
    state: Arc<AppState>,
    thread_id: String,
    terminal_id: String,
) {
    let mut buffer = [0u8; 4096];
    loop {
        let Ok(read) = stream.read(&mut buffer).await else {
            break;
        };
        if read == 0 {
            break;
        }
        let data = String::from_utf8_lossy(&buffer[..read]).to_string();
        history.lock().await.push_str(&data);
        let _ = state
            .emit_terminal_event(json!({
                "threadId": thread_id,
                "terminalId": terminal_id,
                "createdAt": now_iso(),
                "type": "output",
                "data": data
            }))
            .await;
    }
}

async fn watch_terminal_process(
    state: Arc<AppState>,
    key: String,
    thread_id: String,
    terminal_id: String,
    mut child: Child,
    mut kill_rx: oneshot::Receiver<()>,
) {
    let status = tokio::select! {
        _ = &mut kill_rx => {
            let _ = child.kill().await;
            child.wait().await.ok()
        }
        status = child.wait() => status.ok(),
    };
    state.terminals.lock().await.remove(&key);
    let _ = state
        .emit_terminal_event(json!({
            "threadId": thread_id,
            "terminalId": terminal_id,
            "createdAt": now_iso(),
            "type": "exited",
            "exitCode": status.and_then(|entry| entry.code()),
            "exitSignal": Value::Null
        }))
        .await;
}

fn terminal_key(body: &serde_json::Map<String, Value>) -> String {
    format!(
        "{}:{}",
        body.get("threadId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        body.get("terminalId")
            .and_then(Value::as_str)
            .unwrap_or("default")
    )
}
