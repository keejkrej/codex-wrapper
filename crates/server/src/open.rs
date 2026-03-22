use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::util::required_string_from_object;

pub(crate) fn open_in_editor(body: &serde_json::Map<String, Value>) -> Result<()> {
    let cwd = required_string_from_object(body, "cwd")?;
    let editor = required_string_from_object(body, "editor")?;
    let mut command = if editor == "file-manager" {
        if cfg!(target_os = "windows") {
            let mut process = std::process::Command::new("explorer");
            process.arg(&cwd);
            process
        } else if cfg!(target_os = "macos") {
            let mut process = std::process::Command::new("open");
            process.arg(&cwd);
            process
        } else {
            let mut process = std::process::Command::new("xdg-open");
            process.arg(&cwd);
            process
        }
    } else {
        let binary = match editor.as_str() {
            "cursor" => "cursor",
            "vscode" => "code",
            "zed" => "zed",
            "antigravity" => "agy",
            _ => return Err(anyhow!("Unsupported editor")),
        };
        let mut process = std::process::Command::new(binary);
        process.arg(&cwd);
        process
    };
    command.spawn()?;
    Ok(())
}
