use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;

use crate::error::AppResult;

const STORE_FILE: &str = "settings.json";
const KEY_QUICKSWITCH_MODE: &str = "quickswitch_mode";
const KEY_QUICKSWITCH_ITEMS: &str = "quickswitch_items";
const KEY_HIDE_DONE_TASKS: &str = "hide_done_tasks";
const KEY_DEFAULT_TASK: &str = "default_task";

// ---------------------------------------------------------------------------
// Quick-switch item (manual-mode pinned task)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickSwitchItem {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
    /// "main" (4 large) or "small" (3 compact)
    pub slot: String,
}

// ---------------------------------------------------------------------------
// Default task (for untracked "private todo" time)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultTaskConfig {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_quickswitch_mode(
    app_handle: tauri::AppHandle,
) -> AppResult<String> {
    let mode = app_handle
        .store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(KEY_QUICKSWITCH_MODE))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "auto".to_string());
    Ok(mode)
}

#[tauri::command]
pub async fn set_quickswitch_mode(
    mode: String,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    if let Ok(store) = app_handle.store(STORE_FILE) {
        store.set(KEY_QUICKSWITCH_MODE, serde_json::json!(mode));
        let _ = store.save();
    }
    log::info!("Quick-switch mode set to: {mode}");
    Ok(())
}

#[tauri::command]
pub async fn get_quickswitch_items(
    app_handle: tauri::AppHandle,
) -> AppResult<Vec<QuickSwitchItem>> {
    let items = app_handle
        .store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(KEY_QUICKSWITCH_ITEMS))
        .and_then(|v| serde_json::from_value::<Vec<QuickSwitchItem>>(v.clone()).ok())
        .unwrap_or_default();
    Ok(items)
}

#[tauri::command]
pub async fn set_quickswitch_items(
    items: Vec<QuickSwitchItem>,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    if let Ok(store) = app_handle.store(STORE_FILE) {
        store.set(KEY_QUICKSWITCH_ITEMS, serde_json::json!(items));
        let _ = store.save();
    }
    log::info!("Quick-switch items saved ({} items)", items.len());
    Ok(())
}

#[tauri::command]
pub async fn get_hide_done_tasks(
    app_handle: tauri::AppHandle,
) -> AppResult<bool> {
    let hide = app_handle
        .store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(KEY_HIDE_DONE_TASKS))
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // default: hide done tasks
    Ok(hide)
}

#[tauri::command]
pub async fn set_hide_done_tasks(
    hide: bool,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    if let Ok(store) = app_handle.store(STORE_FILE) {
        store.set(KEY_HIDE_DONE_TASKS, serde_json::json!(hide));
        let _ = store.save();
    }
    log::info!("Hide done tasks set to: {hide}");
    Ok(())
}

#[tauri::command]
pub async fn get_quick_switch_entries(
    app: tauri::AppHandle,
) -> AppResult<Vec<crate::reminder::QuickSwitchEntry>> {
    Ok(crate::reminder::build_quick_switch_entries_public(&app).await)
}

// ---------------------------------------------------------------------------
// Default task commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_default_task(
    app_handle: tauri::AppHandle,
) -> AppResult<Option<DefaultTaskConfig>> {
    let task = app_handle
        .store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(KEY_DEFAULT_TASK))
        .and_then(|v| serde_json::from_value::<DefaultTaskConfig>(v.clone()).ok());
    Ok(task)
}

#[tauri::command]
pub async fn set_default_task(
    task_id: i64,
    task_name: String,
    project_id: i64,
    project_name: String,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    let config = DefaultTaskConfig { task_id, task_name: task_name.clone(), project_id, project_name };
    if let Ok(store) = app_handle.store(STORE_FILE) {
        store.set(KEY_DEFAULT_TASK, serde_json::json!(config));
        let _ = store.save();
    }
    log::info!("Default task set to: {} (id={})", task_name, task_id);
    Ok(())
}

#[tauri::command]
pub async fn clear_default_task(
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    if let Ok(store) = app_handle.store(STORE_FILE) {
        let _ = store.delete(KEY_DEFAULT_TASK);
        let _ = store.save();
    }
    log::info!("Default task cleared");
    Ok(())
}
