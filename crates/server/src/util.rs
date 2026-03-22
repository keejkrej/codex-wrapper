use std::path::{Component, Path};

use anyhow::{anyhow, Result};
use chrono::{SecondsFormat, Utc};
use serde_json::Value;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn required_string(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("{key} is required"))
}

pub fn required_string_from_object(
    value: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("{key} is required"))
}

pub fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn ensure_relative_path(path: &str) -> Result<()> {
    for component in Path::new(path).components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(anyhow!(
                    "Workspace file path must stay within the project root"
                ))
            }
            _ => {}
        }
    }
    Ok(())
}
