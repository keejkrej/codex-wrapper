use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::util::now_iso;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Default)]
struct CommandResult {
    stdout: String,
    stderr: String,
    success: bool,
}

pub(crate) async fn provider_statuses() -> Value {
    json!([
        check_codex_provider_status().await,
        check_claude_provider_status().await
    ])
}

async fn check_codex_provider_status() -> Value {
    let checked_at = now_iso();
    let version = run_command("codex", &["--version"]).await;
    let Some(version) = version else {
        return json!({
            "provider": "codex",
            "status": "error",
            "available": false,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": "Codex CLI (`codex`) is not installed, failed to start, or timed out."
        });
    };
    if !version.success {
        return json!({
            "provider": "codex",
            "status": "error",
            "available": false,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": detail_message(
                "Codex CLI is installed but failed to run.",
                &version.stdout,
                &version.stderr
            )
        });
    }

    let auth = run_command("codex", &["login", "status"]).await;
    match auth {
        None => json!({
            "provider": "codex",
            "status": "warning",
            "available": true,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": "Could not verify Codex authentication status."
        }),
        Some(result) => {
            let combined = format!("{}\n{}", result.stdout, result.stderr).to_lowercase();
            if combined.contains("not logged in")
                || combined.contains("login required")
                || combined.contains("authentication required")
                || combined.contains("run `codex login`")
                || combined.contains("run codex login")
            {
                json!({
                    "provider": "codex",
                    "status": "error",
                    "available": true,
                    "authStatus": "unauthenticated",
                    "checkedAt": checked_at,
                    "message": "Codex CLI is not authenticated. Run `codex login` and try again."
                })
            } else if result.success {
                json!({
                    "provider": "codex",
                    "status": "ready",
                    "available": true,
                    "authStatus": "authenticated",
                    "checkedAt": checked_at
                })
            } else {
                json!({
                    "provider": "codex",
                    "status": "warning",
                    "available": true,
                    "authStatus": "unknown",
                    "checkedAt": checked_at,
                    "message": detail_message(
                        "Could not verify Codex authentication status.",
                        &result.stdout,
                        &result.stderr
                    )
                })
            }
        }
    }
}

async fn check_claude_provider_status() -> Value {
    let checked_at = now_iso();
    let version = run_command("claude", &["--version"]).await;
    let Some(version) = version else {
        return json!({
            "provider": "claudeAgent",
            "status": "error",
            "available": false,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": "Claude Agent CLI (`claude`) is not installed, failed to start, or timed out."
        });
    };
    if !version.success {
        return json!({
            "provider": "claudeAgent",
            "status": "error",
            "available": false,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": detail_message(
                "Claude Agent CLI is installed but failed to run.",
                &version.stdout,
                &version.stderr
            )
        });
    }

    let auth = run_command("claude", &["auth", "status"]).await;
    match auth {
        None => json!({
            "provider": "claudeAgent",
            "status": "warning",
            "available": true,
            "authStatus": "unknown",
            "checkedAt": checked_at,
            "message": "Could not verify Claude authentication status."
        }),
        Some(result) => {
            let combined = format!("{}\n{}", result.stdout, result.stderr).to_lowercase();
            if combined.contains("not logged in")
                || combined.contains("login required")
                || combined.contains("authentication required")
                || combined.contains("run `claude login`")
                || combined.contains("run claude login")
            {
                json!({
                    "provider": "claudeAgent",
                    "status": "error",
                    "available": true,
                    "authStatus": "unauthenticated",
                    "checkedAt": checked_at,
                    "message": "Claude is not authenticated. Run `claude auth login` and try again."
                })
            } else if result.success {
                json!({
                    "provider": "claudeAgent",
                    "status": "ready",
                    "available": true,
                    "authStatus": "authenticated",
                    "checkedAt": checked_at
                })
            } else {
                json!({
                    "provider": "claudeAgent",
                    "status": "warning",
                    "available": true,
                    "authStatus": "unknown",
                    "checkedAt": checked_at,
                    "message": detail_message(
                        "Could not verify Claude authentication status.",
                        &result.stdout,
                        &result.stderr
                    )
                })
            }
        }
    }
}

async fn run_command(program: &str, args: &[&str]) -> Option<CommandResult> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(program);
        command.args(args);
        command
    } else {
        let mut command = Command::new(program);
        command.args(args);
        command
    };

    let output = timeout(DEFAULT_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    Some(CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        success: output.status.success(),
    })
}

fn detail_message(prefix: &str, stdout: &str, stderr: &str) -> String {
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        ""
    };
    if detail.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} {detail}")
    }
}
