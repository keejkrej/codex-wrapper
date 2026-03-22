use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::util::{normalize_path, required_string_from_object};

fn run_git(cwd: &str, args: &[&str]) -> Result<std::process::Output> {
    Ok(std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?)
}

fn run_gh(args: &[&str]) -> Result<std::process::Output> {
    Ok(std::process::Command::new("gh")
        .args(args)
        .output()
        .with_context(|| format!("failed to run gh {}", args.join(" ")))?)
}

fn git_is_repo(cwd: &str) -> bool {
    run_git(cwd, &["rev-parse", "--is-inside-work-tree"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn current_branch_name(cwd: &str) -> Result<Option<String>> {
    let branch = String::from_utf8(run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?.stdout)?
        .trim()
        .to_string();
    if branch == "HEAD" || branch.is_empty() {
        Ok(None)
    } else {
        Ok(Some(branch))
    }
}

fn git_origin_repo(cwd: &str) -> Result<String> {
    let remote = String::from_utf8(run_git(cwd, &["remote", "get-url", "origin"])?.stdout)?
        .trim()
        .trim_end_matches(".git")
        .to_string();
    if let Some(rest) = remote.strip_prefix("https://github.com/") {
        return Ok(rest.to_string());
    }
    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        return Ok(rest.to_string());
    }
    if let Some(rest) = remote.strip_prefix("ssh://git@github.com/") {
        return Ok(rest.to_string());
    }
    Err(anyhow!("Origin remote is not a GitHub repository"))
}

fn default_branch_name(cwd: &str) -> String {
    let remote_head = run_git(cwd, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .and_then(|value| value.rsplit('/').next().map(ToOwned::to_owned));
    remote_head.unwrap_or_else(|| "main".to_string())
}

fn sanitize_branch_component(input: &str) -> String {
    let mut value = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while value.contains("//") {
        value = value.replace("//", "/");
    }
    value.trim_matches('-').trim_matches('/').to_string()
}

fn unique_worktree_path(cwd: &str, branch: &str) -> PathBuf {
    let repo_name = PathBuf::from(cwd)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repo")
        .to_string();
    let parent = PathBuf::from(cwd)
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(cwd));
    let branch_tail = branch.rsplit('/').next().unwrap_or(branch);
    let mut candidate = parent.join(format!("{repo_name}-{branch_tail}"));
    let mut counter = 2usize;
    while candidate.exists() {
        candidate = parent.join(format!("{repo_name}-{branch_tail}-{counter}"));
        counter += 1;
    }
    candidate
}

fn parse_pull_request_reference(reference: &str) -> Result<(Option<String>, u64)> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Pull request reference is required"));
    }
    if let Some(number) = trimmed.strip_prefix('#').or(Some(trimmed)) {
        if let Ok(number) = number.parse::<u64>() {
            return Ok((None, number));
        }
    }
    let pattern = regex_like_github_url(trimmed)?;
    Ok(pattern)
}

fn regex_like_github_url(input: &str) -> Result<(Option<String>, u64)> {
    let marker = "https://github.com/";
    let Some(rest) = input.strip_prefix(marker) else {
        return Err(anyhow!("Unsupported pull request reference"));
    };
    let parts = rest.split('/').collect::<Vec<_>>();
    if parts.len() < 4 || parts[2] != "pull" {
        return Err(anyhow!("Unsupported pull request reference"));
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    let number_part = parts[3]
        .split(['?', '#'])
        .next()
        .ok_or_else(|| anyhow!("Invalid pull request URL"))?;
    let number = number_part
        .parse::<u64>()
        .map_err(|_| anyhow!("Invalid pull request URL"))?;
    Ok((Some(repo), number))
}

fn resolve_pull_request(cwd: &str, reference: &str) -> Result<Value> {
    let (repo_from_reference, number) = parse_pull_request_reference(reference)?;
    let repo = repo_from_reference.unwrap_or(git_origin_repo(cwd)?);
    let fallback_base_branch = default_branch_name(cwd);
    let endpoint = format!("repos/{repo}/pulls/{number}");
    let output = run_gh(&["api", endpoint.as_str()])?;
    if !output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    let payload: Value = serde_json::from_slice(&output.stdout)?;
    let state = if payload.get("state").and_then(Value::as_str) == Some("closed")
        && payload
            .get("merged_at")
            .is_some_and(|value| !value.is_null())
    {
        "merged"
    } else {
        payload
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("open")
    };
    Ok(json!({
        "pullRequest": {
            "number": payload.get("number").and_then(Value::as_u64).unwrap_or(number),
            "title": payload.get("title").and_then(Value::as_str).unwrap_or_default(),
            "url": payload.get("html_url").and_then(Value::as_str).unwrap_or_default(),
            "baseBranch": payload.get("base").and_then(|value| value.get("ref")).and_then(Value::as_str).unwrap_or(&fallback_base_branch),
            "headBranch": payload.get("head").and_then(|value| value.get("ref")).and_then(Value::as_str).unwrap_or_default(),
            "state": state
        }
    }))
}

pub(crate) fn git_status(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    if !git_is_repo(&cwd) {
        return Ok(json!({
            "branch": Value::Null,
            "hasWorkingTreeChanges": false,
            "workingTree": { "files": [], "insertions": 0, "deletions": 0 },
            "hasUpstream": false,
            "aheadCount": 0,
            "behindCount": 0,
            "pr": Value::Null
        }));
    }
    let branch = current_branch_name(&cwd)?;
    let porcelain = String::from_utf8(run_git(&cwd, &["status", "--porcelain"])?.stdout)?;
    let upstream_output = run_git(
        &cwd,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )?;
    let has_upstream = upstream_output.status.success();
    let (ahead, behind) = if has_upstream {
        let counts = String::from_utf8(
            run_git(
                &cwd,
                &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            )?
            .stdout,
        )?;
        let mut parts = counts.split_whitespace();
        let behind = parts.next().unwrap_or("0").parse::<u64>().unwrap_or(0);
        let ahead = parts.next().unwrap_or("0").parse::<u64>().unwrap_or(0);
        (ahead, behind)
    } else {
        (0, 0)
    };
    let files = porcelain
        .lines()
        .filter_map(|line| {
            let path = line.get(3..)?.trim();
            if path.is_empty() {
                None
            } else {
                Some(json!({ "path": path, "insertions": 0, "deletions": 0 }))
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "branch": branch,
        "hasWorkingTreeChanges": !porcelain.trim().is_empty(),
        "workingTree": { "files": files, "insertions": 0, "deletions": 0 },
        "hasUpstream": has_upstream,
        "aheadCount": ahead,
        "behindCount": behind,
        "pr": Value::Null
    }))
}

pub(crate) fn git_list_branches(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    if !git_is_repo(&cwd) {
        return Ok(json!({ "branches": [], "isRepo": false, "hasOriginRemote": false }));
    }
    let output = String::from_utf8(
        run_git(
            &cwd,
            &[
                "branch",
                "--format=%(refname:short)|%(HEAD)|%(worktreepath)",
            ],
        )?
        .stdout,
    )?;
    let current_branch = output
        .lines()
        .find(|line| line.contains("|*|"))
        .and_then(|line| line.split('|').next())
        .unwrap_or("");
    let default_branch = default_branch_name(&cwd);
    let branches = output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('|');
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let head = parts.next().unwrap_or("").trim() == "*";
            let worktree_path = parts.next().unwrap_or("").trim();
            Some(json!({
                "name": name,
                "current": head,
                "isDefault": name == default_branch || name == current_branch,
                "worktreePath": if worktree_path.is_empty() { Value::Null } else { json!(normalize_path(Path::new(worktree_path))) }
            }))
        })
        .collect::<Vec<_>>();
    let has_origin_remote = run_git(&cwd, &["remote", "get-url", "origin"])
        .map(|output| output.status.success())
        .unwrap_or(false);
    Ok(json!({ "branches": branches, "isRepo": true, "hasOriginRemote": has_origin_remote }))
}

pub(crate) fn git_pull(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let before = git_status(body)?;
    let output = run_git(&cwd, &["pull", "--ff-only"])?;
    if !output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    Ok(json!({
        "status": if before["behindCount"].as_u64().unwrap_or_default() == 0 { "skipped_up_to_date" } else { "pulled" },
        "branch": before["branch"].clone(),
        "upstreamBranch": Value::Null
    }))
}

pub(crate) fn git_simple(body: &serde_json::Map<String, Value>, args: &[&str]) -> Result<()> {
    let cwd = required_string_from_object(body, "cwd")?;
    let output = run_git(&cwd, args)?;
    if !output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }
    Ok(())
}

pub(crate) fn git_create_worktree(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let branch = required_string_from_object(body, "branch")?;
    let new_branch = body
        .get("newBranch")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let target_path = body
        .get("path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            let base = PathBuf::from(&cwd)
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&cwd));
            normalize_path(&base.join(new_branch.clone().unwrap_or_else(|| branch.clone())))
        });

    let output = if let Some(new_branch) = new_branch.as_deref() {
        run_git(
            &cwd,
            &["worktree", "add", "-b", new_branch, &target_path, &branch],
        )?
    } else {
        run_git(&cwd, &["worktree", "add", &target_path, &branch])?
    };
    if !output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }

    Ok(json!({
        "worktree": {
            "path": target_path,
            "branch": new_branch.unwrap_or(branch)
        }
    }))
}

pub(crate) fn git_remove_worktree(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let path = required_string_from_object(body, "path")?;
    let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);

    let output = if force {
        run_git(&cwd, &["worktree", "remove", "--force", &path])?
    } else {
        run_git(&cwd, &["worktree", "remove", &path])?
    };
    if !output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()));
    }

    Ok(Value::Null)
}

pub(crate) fn git_resolve_pull_request(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let reference = required_string_from_object(body, "reference")?;
    resolve_pull_request(&cwd, &reference)
}

pub(crate) fn git_prepare_pull_request_thread(
    body: &serde_json::Map<String, Value>,
) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let reference = required_string_from_object(body, "reference")?;
    let mode = required_string_from_object(body, "mode")?;
    let resolved = resolve_pull_request(&cwd, &reference)?;
    let pull_request = resolved
        .get("pullRequest")
        .cloned()
        .ok_or_else(|| anyhow!("Failed to resolve pull request"))?;
    let number = pull_request
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Failed to resolve pull request number"))?;
    let head_branch = pull_request
        .get("headBranch")
        .and_then(Value::as_str)
        .unwrap_or("pr");
    let local_branch = format!("pr/{number}-{}", sanitize_branch_component(head_branch));
    let fetch_ref = format!("+refs/pull/{number}/head:refs/heads/{local_branch}");
    let fetch_output = run_git(&cwd, &["fetch", "origin", &fetch_ref])?;
    if !fetch_output.status.success() {
        return Err(anyhow!(String::from_utf8_lossy(&fetch_output.stderr)
            .trim()
            .to_string()));
    }

    let worktree_path = if mode == "local" {
        let checkout_output = run_git(&cwd, &["checkout", &local_branch])?;
        if !checkout_output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&checkout_output.stderr)
                .trim()
                .to_string()));
        }
        Value::Null
    } else {
        let path = unique_worktree_path(&cwd, &local_branch);
        let path_str = normalize_path(&path);
        let output = run_git(&cwd, &["worktree", "add", &path_str, &local_branch])?;
        if !output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string()));
        }
        json!(path_str)
    };

    Ok(json!({
        "pullRequest": pull_request,
        "branch": local_branch,
        "worktreePath": worktree_path
    }))
}

pub(crate) fn git_run_stacked_action(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = required_string_from_object(body, "cwd")?;
    let action = required_string_from_object(body, "action")?;
    let commit_message = body
        .get("commitMessage")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let feature_branch = body
        .get("featureBranch")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let file_paths = body
        .get("filePaths")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut branch_status = json!({ "status": "skipped_not_requested" });
    let initial_branch = current_branch_name(&cwd)?.ok_or_else(|| anyhow!("Detached HEAD"))?;
    let default_branch = default_branch_name(&cwd);
    let mut branch_name = initial_branch.clone();
    if feature_branch && (initial_branch == default_branch || initial_branch == "master") {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        branch_name = format!("codex/{suffix}");
        let output = run_git(&cwd, &["checkout", "-b", &branch_name])?;
        if !output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string()));
        }
        branch_status = json!({ "status": "created", "name": branch_name });
    }

    if file_paths.is_empty() {
        let add_output = run_git(&cwd, &["add", "-A"])?;
        if !add_output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&add_output.stderr)
                .trim()
                .to_string()));
        }
    } else {
        let mut command = std::process::Command::new("git");
        command.current_dir(&cwd).arg("add").arg("--");
        for path in &file_paths {
            command.arg(path);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string()));
        }
    }

    let has_staged_changes = !run_git(&cwd, &["diff", "--cached", "--quiet"])
        .map(|output| output.status.success())
        .unwrap_or(false);
    let mut commit_status = json!({ "status": "skipped_no_changes" });
    if has_staged_changes {
        let subject = commit_message.unwrap_or_else(|| {
            if file_paths.len() == 1 {
                format!("Update {}", file_paths[0])
            } else {
                "Update files".to_string()
            }
        });
        let commit_output = run_git(&cwd, &["commit", "-m", &subject])?;
        if !commit_output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&commit_output.stderr)
                .trim()
                .to_string()));
        }
        let sha = String::from_utf8(run_git(&cwd, &["rev-parse", "HEAD"])?.stdout)?
            .trim()
            .to_string();
        let subject = String::from_utf8(run_git(&cwd, &["log", "-1", "--pretty=%s"])?.stdout)?
            .trim()
            .to_string();
        commit_status = json!({
            "status": "created",
            "commitSha": sha,
            "subject": subject
        });
    }

    let mut push_status = json!({ "status": "skipped_not_requested" });
    if action == "commit_push" || action == "commit_push_pr" {
        let status = git_status(&serde_json::Map::from_iter([(
            "cwd".to_string(),
            json!(cwd.clone()),
        )]))?;
        let has_upstream = status["hasUpstream"].as_bool().unwrap_or(false);
        let ahead = status["aheadCount"].as_u64().unwrap_or_default();
        if ahead == 0 {
            push_status = json!({ "status": "skipped_up_to_date" });
        } else {
            let output = if has_upstream {
                run_git(&cwd, &["push"])?
            } else {
                run_git(&cwd, &["push", "-u", "origin", &branch_name])?
            };
            if !output.status.success() {
                return Err(anyhow!(String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .to_string()));
            }
            let upstream_branch = run_git(
                &cwd,
                &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            )
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_string());
            push_status = json!({
                "status": "pushed",
                "branch": branch_name,
                "upstreamBranch": upstream_branch,
                "setUpstream": !has_upstream
            });
        }
    }

    let mut pr_status = json!({ "status": "skipped_not_requested" });
    if action == "commit_push_pr" {
        let repo = git_origin_repo(&cwd)?;
        let existing_output = run_gh(&[
            "pr",
            "list",
            "--repo",
            &repo,
            "--head",
            &branch_name,
            "--state",
            "all",
            "--json",
            "number,title,url,baseRefName,headRefName,state",
            "--limit",
            "1",
        ])?;
        if !existing_output.status.success() {
            return Err(anyhow!(String::from_utf8_lossy(&existing_output.stderr)
                .trim()
                .to_string()));
        }
        let existing: Value = serde_json::from_slice(&existing_output.stdout)?;
        let existing_pr = existing
            .as_array()
            .and_then(|entries| entries.first())
            .cloned();
        let pr = if let Some(pr) = existing_pr {
            pr_status = json!({
                "status": "opened_existing",
                "url": pr.get("url").cloned().unwrap_or(Value::Null),
                "number": pr.get("number").cloned().unwrap_or(Value::Null),
                "baseBranch": pr.get("baseRefName").cloned().unwrap_or(Value::Null),
                "headBranch": pr.get("headRefName").cloned().unwrap_or(Value::Null),
                "title": pr.get("title").cloned().unwrap_or(Value::Null)
            });
            None
        } else {
            let title = commit_status
                .get("subject")
                .and_then(Value::as_str)
                .unwrap_or("Update files");
            let base = default_branch_name(&cwd);
            let create_output = run_gh(&[
                "pr",
                "create",
                "--repo",
                &repo,
                "--head",
                &branch_name,
                "--base",
                &base,
                "--title",
                title,
                "--body",
                "",
            ])?;
            if !create_output.status.success() {
                return Err(anyhow!(String::from_utf8_lossy(&create_output.stderr)
                    .trim()
                    .to_string()));
            }
            let pr_url = String::from_utf8(create_output.stdout)?.trim().to_string();
            Some((pr_url, base, title.to_string()))
        };
        if let Some((pr_url, base, title)) = pr {
            let view_output = run_gh(&[
                "pr",
                "view",
                &branch_name,
                "--repo",
                &repo,
                "--json",
                "number,headRefName",
            ])?;
            let view: Value = if view_output.status.success() {
                serde_json::from_slice(&view_output.stdout)?
            } else {
                Value::Null
            };
            pr_status = json!({
                "status": "created",
                "url": pr_url,
                "number": view.get("number").cloned().unwrap_or(Value::Null),
                "baseBranch": base,
                "headBranch": view.get("headRefName").cloned().unwrap_or(json!(branch_name)),
                "title": title
            });
        }
    }

    Ok(json!({
        "action": action,
        "branch": branch_status,
        "commit": commit_status,
        "push": push_status,
        "pr": pr_status
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_pull_request_reference() {
        assert_eq!(parse_pull_request_reference("#42").unwrap(), (None, 42));
        assert_eq!(parse_pull_request_reference("42").unwrap(), (None, 42));
    }

    #[test]
    fn parses_url_pull_request_reference() {
        assert_eq!(
            parse_pull_request_reference("https://github.com/pingdotgg/t3code/pull/42").unwrap(),
            (Some("pingdotgg/t3code".to_string()), 42)
        );
    }
}
