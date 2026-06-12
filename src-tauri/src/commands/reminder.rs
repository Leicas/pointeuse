use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};
use crate::reminder::{self, ReminderPayload};
use crate::state::AppState;

const STORE_FILE: &str = "settings.json";
const KEY_REMINDER_INTERVAL: &str = "reminder_interval_minutes";

const VALID_INTERVALS: &[u64] = &[0, 5, 10, 15, 30, 60, 90, 120, 180, 240];

#[tauri::command]
pub async fn set_reminder_interval(
    minutes: u64,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    if !VALID_INTERVALS.contains(&minutes) {
        return Err(AppError::General(format!(
            "Invalid reminder interval: {minutes}. Must be one of {:?}",
            VALID_INTERVALS
        )));
    }

    {
        let mut reminder = state.reminder.lock().unwrap();
        reminder.interval_minutes = minutes;
    }

    // Persist to store
    match app_handle.store(STORE_FILE) {
        Ok(store) => {
            store.set(KEY_REMINDER_INTERVAL, serde_json::json!(minutes));
            if let Err(e) = store.save() {
                log::error!("Failed to save reminder interval to store: {e}");
            }
        }
        Err(e) => log::error!("Could not open store to save reminder interval: {e}"),
    }

    log::info!("Reminder interval set to {minutes} minutes");

    // Mobile: reschedule backup reminder with new interval
    #[cfg(mobile)]
    {
        let sched_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            crate::reminder::schedule_next_reminder(&sched_app).await;
        });
    }

    Ok(())
}

#[tauri::command]
pub async fn get_reminder_interval(
    state: tauri::State<'_, AppState>,
) -> AppResult<u64> {
    let reminder = state.reminder.lock().unwrap();
    Ok(reminder.interval_minutes)
}

#[tauri::command]
pub async fn dismiss_idle_reminder(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    {
        let mut reminder = state.reminder.lock().unwrap();
        reminder.popup_showing = false;
        reminder.reset_elapsed = true;
    }
    log::info!("Idle reminder dismissed");

    // Mobile: schedule next backup reminder
    #[cfg(mobile)]
    {
        let sched_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            crate::reminder::schedule_next_reminder(&sched_app).await;
        });
    }
    #[cfg(desktop)]
    let _ = &app_handle;

    Ok(())
}

#[tauri::command]
pub async fn test_reminder_popup(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    let info = {
        let timer = state.timer.lock().unwrap();
        timer.get_state()
    };

    let quick_switch = reminder::build_quick_switch_entries_public(&app_handle).await;

    let payload = ReminderPayload {
        task_id: info.task_id.unwrap_or(0),
        task_name: info.task_name.unwrap_or_else(|| "Test Task".into()),
        project_name: info.project_name.unwrap_or_else(|| "Test Project".into()),
        elapsed_secs: info.elapsed_secs,
        quick_switch,
    };

    reminder::show_reminder_window(&app_handle, &payload);

    log::info!("Test reminder popup shown");
    Ok(())
}

/// Get the current reminder notification channel (silent/normal/urgent).
#[tauri::command]
pub async fn get_reminder_channel(app_handle: tauri::AppHandle) -> AppResult<String> {
    use tauri_plugin_store::StoreExt;

    let channel = app_handle
        .store(STORE_FILE)
        .ok()
        .and_then(|s| s.get("reminder_channel"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "reminder_urgent_v2".to_string());
    Ok(channel)
}

/// Set the reminder notification channel (controls sound/vibration).
#[tauri::command]
pub async fn set_reminder_channel(
    channel: String,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    use tauri_plugin_store::StoreExt;

    let valid = ["reminder_silent_v2", "reminder_normal_v2", "reminder_urgent_v2"];
    if !valid.contains(&channel.as_str()) {
        return Err(AppError::General(format!("Invalid channel: {channel}")));
    }

    match app_handle.store(STORE_FILE) {
        Ok(store) => {
            store.set("reminder_channel", serde_json::json!(channel));
            if let Err(e) = store.save() {
                log::error!("Failed to save reminder channel: {e}");
            }
        }
        Err(e) => log::error!("Could not open store to save channel: {e}"),
    }

    log::info!("Reminder channel set to {channel}");
    Ok(())
}
