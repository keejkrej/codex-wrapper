use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::util::{ensure_relative_path, normalize_path, required_string_from_object};

pub(crate) fn search_project_entries(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = PathBuf::from(required_string_from_object(body, "cwd")?);
    let query = required_string_from_object(body, "query")?.to_lowercase();
    let limit = body.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let mut entries = Vec::new();
    for entry in WalkDir::new(&cwd)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.path() == cwd {
            continue;
        }
        let relative = match entry.path().strip_prefix(&cwd) {
            Ok(path) => normalize_path(path),
            Err(_) => continue,
        };
        if !relative.to_lowercase().contains(&query) {
            continue;
        }
        let parent_path = Path::new(&relative)
            .parent()
            .map(normalize_path)
            .filter(|value| !value.is_empty() && value != ".");
        entries.push(json!({
            "path": relative,
            "kind": if entry.file_type().is_dir() { "directory" } else { "file" },
            "parentPath": parent_path
        }));
        if entries.len() >= limit {
            break;
        }
    }
    Ok(json!({ "entries": entries, "truncated": false }))
}

pub(crate) fn write_project_file(body: &serde_json::Map<String, Value>) -> Result<Value> {
    let cwd = PathBuf::from(required_string_from_object(body, "cwd")?);
    let relative_path = required_string_from_object(body, "relativePath")?;
    ensure_relative_path(&relative_path)?;
    let contents = body
        .get("contents")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = cwd.join(&relative_path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, contents)?;
    Ok(json!({ "relativePath": relative_path }))
}
