#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use server::{start_server, ServerConfig, ServerHandle};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{MenuBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, State, Theme};

const MENU_ACTION_EVENT: &str = "desktop:menu-action";
const UPDATE_STATE_EVENT: &str = "desktop:update-state";
const MENU_ITEM_OPEN_SETTINGS_ID: &str = "desktop-open-settings";
const MENU_ITEM_CHECK_FOR_UPDATES_ID: &str = "desktop-check-for-updates";

struct DesktopState {
    server: ServerHandle,
    next_context_menu_id: AtomicU64,
    pending_context_menu: Arc<Mutex<Option<PendingContextMenuState>>>,
    update_state: Arc<Mutex<Value>>,
}

struct PendingContextMenuState {
    item_ids_by_menu_id: HashMap<String, String>,
    selected_item_id: Option<String>,
}

#[derive(Clone, serde::Deserialize)]
struct ContextMenuItemPayload {
    id: String,
    label: String,
    destructive: Option<bool>,
}

#[derive(Clone, serde::Deserialize)]
struct ContextMenuPosition {
    x: f64,
    y: f64,
}

impl DesktopState {
    fn new(server: ServerHandle) -> Self {
        Self {
            server,
            next_context_menu_id: AtomicU64::new(1),
            pending_context_menu: Arc::new(Mutex::new(None)),
            update_state: Arc::new(Mutex::new(disabled_update_state())),
        }
    }
}

fn emit_menu_action(app: &AppHandle, action: &str) {
    let _ = app.emit_to("main", MENU_ACTION_EVENT, action);
}

fn current_update_state(state: &DesktopState) -> Value {
    state
        .update_state
        .lock()
        .expect("poisoned update state")
        .clone()
}

fn emit_update_state(app: &AppHandle, state: &DesktopState) {
    let _ = app.emit_to("main", UPDATE_STATE_EVENT, current_update_state(state));
}

fn build_disabled_update_action(state: &DesktopState) -> Value {
    json!({
        "accepted": false,
        "completed": false,
        "state": current_update_state(state),
    })
}

fn show_updates_unavailable_dialog() {
    let _ = MessageDialog::new()
        .set_level(MessageLevel::Info)
        .set_title("Updates unavailable")
        .set_description("Automatic updates are not available in the Tauri shell yet.")
        .set_buttons(MessageButtons::Ok)
        .show();
}

fn build_application_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    let file_menu = SubmenuBuilder::new(app, "File")
        .text(MENU_ITEM_OPEN_SETTINGS_ID, "Settings...")
        .build()?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .text(MENU_ITEM_CHECK_FOR_UPDATES_ID, "Check for Updates...")
        .build()?;

    MenuBuilder::new(app)
        .item(&file_menu)
        .item(&help_menu)
        .build()
}

fn handle_menu_event(app: &AppHandle, event_id: &str) {
    match event_id {
        MENU_ITEM_OPEN_SETTINGS_ID => emit_menu_action(app, "open-settings"),
        MENU_ITEM_CHECK_FOR_UPDATES_ID => {
            if let Some(state) = app.try_state::<DesktopState>() {
                emit_update_state(app, state.inner());
            }
            show_updates_unavailable_dialog();
        }
        _ => {
            if let Some(state) = app.try_state::<DesktopState>() {
                let mut pending = state
                    .pending_context_menu
                    .lock()
                    .expect("poisoned context menu state");
                if let Some(active_menu) = pending.as_mut() {
                    if let Some(selected_item_id) =
                        active_menu.item_ids_by_menu_id.get(event_id).cloned()
                    {
                        active_menu.selected_item_id = Some(selected_item_id);
                    }
                }
            }
        }
    }
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
fn get_update_state(app: AppHandle, state: State<'_, DesktopState>) -> Value {
    emit_update_state(&app, state.inner());
    current_update_state(state.inner())
}

#[tauri::command]
fn download_update(app: AppHandle, state: State<'_, DesktopState>) -> Value {
    emit_update_state(&app, state.inner());
    build_disabled_update_action(state.inner())
}

#[tauri::command]
fn install_update(app: AppHandle, state: State<'_, DesktopState>) -> Value {
    emit_update_state(&app, state.inner());
    build_disabled_update_action(state.inner())
}

#[tauri::command]
fn show_context_menu(
    app: AppHandle,
    state: State<'_, DesktopState>,
    items: Vec<ContextMenuItemPayload>,
    position: Option<ContextMenuPosition>,
) -> Result<Option<String>, String> {
    if items.is_empty() {
        return Ok(None);
    }

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is unavailable".to_string())?;
    let request_id = state.next_context_menu_id.fetch_add(1, Ordering::SeqCst);
    let mut item_ids_by_menu_id = HashMap::new();
    let mut builder = MenuBuilder::new(&app);
    let mut inserted_destructive_separator = false;
    let mut added_items = 0_u32;

    for item in items {
        if item.label.trim().is_empty() {
            continue;
        }
        if item.destructive.unwrap_or(false) && !inserted_destructive_separator && added_items > 0 {
            builder = builder.separator();
            inserted_destructive_separator = true;
        }
        let menu_id = format!("context-menu-{request_id}-{added_items}");
        item_ids_by_menu_id.insert(menu_id.clone(), item.id);
        builder = builder.text(menu_id, item.label);
        added_items += 1;
    }

    if added_items == 0 {
        return Ok(None);
    }

    {
        let mut pending = state
            .pending_context_menu
            .lock()
            .expect("poisoned context menu state");
        *pending = Some(PendingContextMenuState {
            item_ids_by_menu_id,
            selected_item_id: None,
        });
    }

    let menu = builder.build().map_err(|error| error.to_string())?;
    match position {
        Some(position) => window
            .popup_menu_at(&menu, LogicalPosition::new(position.x, position.y))
            .map_err(|error| error.to_string())?,
        None => window.popup_menu(&menu).map_err(|error| error.to_string())?,
    }

    let selected_item_id = state
        .pending_context_menu
        .lock()
        .expect("poisoned context menu state")
        .take()
        .and_then(|pending| pending.selected_item_id);

    Ok(selected_item_id)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let server = tauri::async_runtime::block_on(start_server(ServerConfig::desktop(
                "T3 Code",
                std::env::current_dir().unwrap_or_default(),
            )))?;
            let desktop_state = DesktopState::new(server);
            app.manage(desktop_state);
            let menu = build_application_menu(&app.handle())?;
            let _ = app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                handle_menu_event(app, event.id().as_ref());
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("T3 Code (Alpha)");
            }
            if let Some(state) = app.try_state::<DesktopState>() {
                emit_update_state(&app.handle(), state.inner());
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
            install_update,
            show_context_menu
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
