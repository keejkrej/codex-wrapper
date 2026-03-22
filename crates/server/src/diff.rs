use std::process::Stdio;

use serde_json::{json, Value};
use tokio::process::Command;

use crate::state::AppState;

#[derive(Default)]
pub(crate) struct DiffSummary {
    pub diff: String,
    pub files: Vec<Value>,
    pub checkpoint_ref: String,
}

pub(crate) async fn get_thread_diff(
    state: &AppState,
    thread_id: &str,
    from_turn_count: u64,
    to_turn_count: u64,
) -> Value {
    if from_turn_count >= to_turn_count {
        return json!({
            "threadId": thread_id,
            "fromTurnCount": from_turn_count,
            "toTurnCount": to_turn_count,
            "diff": ""
        });
    }

    let cwd = {
        let snapshot = state.snapshot.lock().await;
        let Some(thread) = snapshot
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
        else {
            return json!({
                "threadId": thread_id,
                "fromTurnCount": from_turn_count,
                "toTurnCount": to_turn_count,
                "diff": ""
            });
        };
        thread
            .worktree_path
            .clone()
            .or_else(|| {
                snapshot
                    .projects
                    .iter()
                    .find(|project| project.id == thread.project_id)
                    .map(|project| project.workspace_root.clone())
            })
            .unwrap_or_else(|| state.cwd_string())
    };

    let diff = summarize_git_changes(&cwd).await.diff;
    json!({
        "threadId": thread_id,
        "fromTurnCount": from_turn_count,
        "toTurnCount": to_turn_count,
        "diff": diff
    })
}

pub(crate) async fn summarize_thread_changes(state: &AppState, thread_id: &str) -> DiffSummary {
    let cwd = {
        let snapshot = state.snapshot.lock().await;
        let Some(thread) = snapshot
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
        else {
            return DiffSummary::default();
        };
        thread
            .worktree_path
            .clone()
            .or_else(|| {
                snapshot
                    .projects
                    .iter()
                    .find(|project| project.id == thread.project_id)
                    .map(|project| project.workspace_root.clone())
            })
            .unwrap_or_else(|| state.cwd_string())
    };

    summarize_git_changes(&cwd).await
}

async fn summarize_git_changes(cwd: &str) -> DiffSummary {
    let diff = run_git_command(cwd, &["diff", "--no-ext-diff", "--binary", "HEAD", "--"])
        .await
        .unwrap_or_default();
    let numstat = run_git_command(cwd, &["diff", "--numstat", "HEAD", "--"])
        .await
        .unwrap_or_default();
    let checkpoint_ref = run_git_command(cwd, &["rev-parse", "HEAD"])
        .await
        .unwrap_or_else(|| "HEAD".to_string())
        .trim()
        .to_string();
    let files = numstat
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let additions = parts.next()?.parse::<u64>().ok()?;
            let deletions = parts.next()?.parse::<u64>().ok()?;
            let path = parts.next()?.to_string();
            Some(json!({
                "path": path,
                "kind": "modified",
                "additions": additions,
                "deletions": deletions,
            }))
        })
        .collect();

    DiffSummary {
        diff,
        files,
        checkpoint_ref,
    }
}

async fn run_git_command(cwd: &str, args: &[&str]) -> Option<String> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg("git");
        command.args(args);
        command
    } else {
        let mut command = Command::new("git");
        command.args(args);
        command
    };

    let output = command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}
