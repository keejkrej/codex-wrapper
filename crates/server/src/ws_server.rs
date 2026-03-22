use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};

use crate::attachments::{
    normalize_attachment_relative_path, resolve_attachment_path_by_id,
    resolve_attachment_relative_path,
};
use crate::diff::get_thread_diff;
use crate::git::{
    git_create_worktree, git_list_branches, git_prepare_pull_request_thread, git_pull,
    git_remove_worktree, git_resolve_pull_request, git_run_stacked_action, git_simple, git_status,
};
use crate::keybindings::upsert_keybinding;
use crate::open::open_in_editor;
use crate::orchestration::handle_dispatch_command;
use crate::state::AppState;
use crate::terminal::{
    open_terminal, restart_terminal, terminal_clear, terminal_close, terminal_resize,
    terminal_write,
};
use crate::util::required_string_from_object;
use crate::workspace::{search_project_entries, write_project_file};

pub(crate) const WS_CHANNEL_SERVER_WELCOME: &str = "server.welcome";
pub(crate) const WS_CHANNEL_SERVER_CONFIG_UPDATED: &str = "server.configUpdated";
pub(crate) const WS_CHANNEL_TERMINAL_EVENT: &str = "terminal.event";
pub(crate) const WS_CHANNEL_ORCHESTRATION_DOMAIN_EVENT: &str = "orchestration.domainEvent";

pub(crate) fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/api/project-favicon", get(project_favicon))
        .route("/attachments/{*id}", get(attachment_handler))
        .route("/", get(root_handler))
        .route("/{*path}", get(spa_handler))
        .with_state(state)
}

async fn root_handler(State(state): State<Arc<AppState>>) -> Response {
    if let Some(dev_url) = state.config.dev_url.as_deref() {
        return Redirect::temporary(dev_url).into_response();
    }

    if let Some(static_dir) = state.config.static_dir.as_ref() {
        return serve_static_file(static_dir, "index.html").await;
    }

    Html(format!(
        "<html><body><h1>{}</h1><p>Rust server is running.</p></body></html>",
        state.config.app_name
    ))
    .into_response()
}

async fn spa_handler(
    AxumPath(path): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    if let Some(dev_url) = state.config.dev_url.as_deref() {
        let target = if path.is_empty() {
            dev_url.to_string()
        } else {
            format!("{}/{}", dev_url.trim_end_matches('/'), path)
        };
        return Redirect::temporary(&target).into_response();
    }

    if let Some(static_dir) = state.config.static_dir.as_ref() {
        let candidate = if path.is_empty() {
            "index.html"
        } else {
            path.as_str()
        };
        let response = serve_static_file(static_dir, candidate).await;
        if response.status() != StatusCode::NOT_FOUND {
            return response;
        }
        return serve_static_file(static_dir, "index.html").await;
    }

    StatusCode::NOT_FOUND.into_response()
}

async fn serve_static_file(base_dir: &PathBuf, relative_path: &str) -> Response {
    let full_path = base_dir.join(relative_path);
    match tokio::fs::read(&full_path).await {
        Ok(bytes) => {
            let mime = content_type_for_path(relative_path);
            ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn content_type_for_path(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}

async fn project_favicon(Query(_query): Query<HashMap<String, String>>) -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

async fn attachment_handler(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(normalized_relative_path) = normalize_attachment_relative_path(&id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let attachment_path = if normalized_relative_path.contains('.') {
        resolve_attachment_relative_path(&state.config.state_dir, &normalized_relative_path)
    } else {
        resolve_attachment_path_by_id(&state.config.state_dir, &normalized_relative_path)
    };

    let Some(attachment_path) = attachment_path else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match tokio::fs::read(&attachment_path).await {
        Ok(bytes) => {
            let mime = content_type_for_path(attachment_path.to_string_lossy().as_ref());
            ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if !is_authorized(&state, &headers, &query) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| websocket_connection(socket, state))
        .into_response()
}

fn is_authorized(state: &AppState, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    let Some(expected) = state.config.auth_token.as_deref() else {
        return true;
    };

    if let Some(header) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        return header == expected;
    }

    query.get("token").map(String::as_str) == Some(expected)
}

async fn websocket_connection(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut push_rx = state.pushes.subscribe();
    let snapshot = state.snapshot.lock().await;
    let welcome = json!({
        "type": "push",
        "sequence": state.next_sequence(),
        "channel": WS_CHANNEL_SERVER_WELCOME,
        "data": {
            "cwd": state.cwd_string(),
            "projectName": state.config.app_name,
            "bootstrapProjectId": snapshot.projects.first().map(|project| project.id.clone()),
            "bootstrapThreadId": snapshot.threads.first().map(|thread| thread.id.clone())
        }
    });
    drop(snapshot);
    let config_updated = json!({
        "type": "push",
        "sequence": state.next_sequence(),
        "channel": WS_CHANNEL_SERVER_CONFIG_UPDATED,
        "data": state.config_updated_payload().await
    });
    let _ = sender.send(Message::Text(welcome.to_string().into())).await;
    let _ = sender
        .send(Message::Text(config_updated.to_string().into()))
        .await;
    let send_task = tokio::spawn(async move {
        while let Ok(push) = push_rx.recv().await {
            if sender
                .send(Message::Text(push.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let response = match handle_ws_request(&state, &text).await {
            Ok(response) => response,
            Err(error) => err_response("", &error.to_string()),
        };
        let _ = state.pushes.send(response);
    }
    send_task.abort();
}

async fn handle_ws_request(state: &Arc<AppState>, text: &str) -> Result<Value> {
    let request: Value = serde_json::from_str(text).context("invalid websocket request json")?;
    let id = request
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let body = request
        .get("body")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("request body is required"))?;
    let tag = body
        .get("_tag")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("request body tag is required"))?;

    let result = match tag {
        "orchestration.getSnapshot" => state.snapshot_value().await?,
        "orchestration.dispatchCommand" => {
            let command = body
                .get("command")
                .ok_or_else(|| anyhow!("dispatch command payload is required"))?;
            handle_dispatch_command(state.clone(), command).await?
        }
        "orchestration.replayEvents" => {
            let from = body
                .get("fromSequenceExclusive")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            state.replay_events(from).await?
        }
        "orchestration.getTurnDiff" => {
            let thread_id = required_string_from_object(body, "threadId")?;
            let from_turn_count = body
                .get("fromTurnCount")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let to_turn_count = body
                .get("toTurnCount")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            get_thread_diff(state, &thread_id, from_turn_count, to_turn_count).await
        }
        "orchestration.getFullThreadDiff" => {
            let thread_id = required_string_from_object(body, "threadId")?;
            let to_turn_count = body
                .get("toTurnCount")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            get_thread_diff(state, &thread_id, 0, to_turn_count).await
        }
        "server.getConfig" => state.server_config_value().await,
        "server.upsertKeybinding" => {
            let key = required_string_from_object(body, "key")?;
            let command = required_string_from_object(body, "command")?;
            let when = body
                .get("when")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let (keybindings, issues) = upsert_keybinding(&state.config.cwd, key, command, when)
                .map_err(|error| anyhow!("Failed to update keybindings: {error}"))?;
            state.emit_config_updated().await?;
            json!({ "keybindings": keybindings, "issues": issues })
        }
        "projects.searchEntries" => search_project_entries(body)?,
        "projects.writeFile" => write_project_file(body)?,
        "shell.openInEditor" => {
            open_in_editor(body)?;
            Value::Null
        }
        "git.status" => git_status(body)?,
        "git.listBranches" => git_list_branches(body)?,
        "git.init" => {
            git_simple(body, &["init"])?;
            Value::Null
        }
        "git.createBranch" => {
            let branch = required_string_from_object(body, "branch")?;
            git_simple(body, &["branch", &branch])?;
            Value::Null
        }
        "git.checkout" => {
            let branch = required_string_from_object(body, "branch")?;
            git_simple(body, &["checkout", &branch])?;
            Value::Null
        }
        "git.pull" => git_pull(body)?,
        "git.createWorktree" => git_create_worktree(body)?,
        "git.removeWorktree" => git_remove_worktree(body)?,
        "git.resolvePullRequest" => git_resolve_pull_request(body)?,
        "git.preparePullRequestThread" => git_prepare_pull_request_thread(body)?,
        "git.runStackedAction" => git_run_stacked_action(body)?,
        "terminal.open" => open_terminal(state.clone(), body).await?,
        "terminal.write" => {
            terminal_write(state.clone(), body).await?;
            Value::Null
        }
        "terminal.resize" => {
            terminal_resize(state.clone(), body).await?;
            Value::Null
        }
        "terminal.clear" => {
            terminal_clear(state.clone(), body).await?;
            Value::Null
        }
        "terminal.restart" => restart_terminal(state.clone(), body).await?,
        "terminal.close" => {
            terminal_close(state.clone(), body).await?;
            Value::Null
        }
        other => {
            return Ok(err_response(
                &id,
                &format!("Unsupported request method: {other}"),
            ))
        }
    };

    Ok(ok_response(&id, result))
}

fn ok_response(id: &str, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

fn err_response(id: &str, message: &str) -> Value {
    json!({ "id": id, "error": { "message": message } })
}
