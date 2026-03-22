#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use tauri::{AppHandle, Manager, Theme};

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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("T3 Code (Alpha)");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pick_folder,
            confirm_dialog,
            set_theme,
            open_external
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
