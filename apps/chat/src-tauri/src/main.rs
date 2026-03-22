#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use server::{start_server, ServerConfig, ServerHandle};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State, Theme};

struct DesktopState {
    server: ServerHandle,
}

#[tauri::command]
fn get_ws_url(state: State<'_, DesktopState>) -> String {
    state.server.ws_url()
}

#[tauri::command]
fn pick_folder() -> Option<String> {
    FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn confirm_dialog(message: String) -> bool {
    matches!(
        MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title("T3 Code")
            .set_description(message)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
}

#[tauri::command]
fn set_theme(app: AppHandle, theme: String) -> Result<(), String> {
    let next_theme = match theme.as_str() {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        "system" => None,
        other => return Err(format!("Unsupported theme: {other}")),
    };

    for window in app.webview_windows().values() {
        let _ = window.set_theme(next_theme);
    }

    Ok(())
}

#[tauri::command]
fn open_external(url: String) -> bool {
    webbrowser::open(&url).is_ok()
}

fn disabled_update_state() -> Value {
    json!({
        "enabled": false,
        "status": "disabled",
        "currentVersion": env!("CARGO_PKG_VERSION"),
        "hostArch": "other",
        "appArch": "other",
        "runningUnderArm64Translation": false,
        "availableVersion": Value::Null,
        "downloadedVersion": Value::Null,
        "downloadPercent": Value::Null,
        "checkedAt": Value::Null,
        "message": "Desktop update support has not been ported to the Tauri shell yet.",
        "errorContext": Value::Null,
        "canRetry": false,
    })
}

#[tauri::command]
fn get_update_state() -> Value {
    disabled_update_state()
}

#[tauri::command]
fn download_update() -> Value {
    json!({
        "accepted": false,
        "completed": false,
        "state": disabled_update_state(),
    })
}

#[tauri::command]
fn install_update() -> Value {
    json!({
        "accepted": false,
        "completed": false,
        "state": disabled_update_state(),
    })
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let server = tauri::async_runtime::block_on(start_server(ServerConfig::desktop(
                "T3 Chat",
                std::env::current_dir().unwrap_or_default(),
            )))?;
            app.manage(DesktopState { server });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("T3 Chat (Alpha)");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_ws_url,
            pick_folder,
            confirm_dialog,
            set_theme,
            open_external,
            get_update_state,
            download_update,
            install_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
