//! Mobile notification management: ongoing timer notification + reminder actions.
//!
//! This module is only compiled on mobile targets.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::time::{self, Duration};

use crate::state::AppState;

// Notification IDs (stable so we can update/remove them)
pub const ONGOING_NOTIFICATION_ID: i32 = 9001;
pub const REMINDER_NOTIFICATION_ID: i32 = 9002;

// Channel IDs
pub const TIMER_CHANNEL_ID: &str = "timer_status";

// Action type IDs
pub const REMINDER_ACTION_TYPE: &str = "reminder_actions";

// ---------------------------------------------------------------------------
// Channel & action type setup (call once at startup)
// ---------------------------------------------------------------------------

// Reminder channel variants by importance level (v2 — bumped to reset cached settings)
pub const REMINDER_CHANNEL_SILENT: &str = "reminder_silent_v2";
pub const REMINDER_CHANNEL_NORMAL: &str = "reminder_normal_v2";
pub const REMINDER_CHANNEL_URGENT: &str = "reminder_urgent_v2";

pub fn setup_notification_channels(app: &AppHandle) {
    use tauri_plugin_notification::{Channel, Importance};

    log::info!("[notif] Setting up notification channels...");

    // Timer ongoing channel (low importance = no sound/vibration, just visible)
    let timer_channel = Channel::builder(TIMER_CHANNEL_ID, "Timer Status")
        .description("Shows the currently running timer".to_string())
        .importance(Importance::Low)
        .vibration(false)
        .build();

    match app.notification().create_channel(timer_channel) {
        Ok(_) => log::info!("[notif] Timer channel created OK"),
        Err(e) => log::error!("[notif] Failed to create timer channel: {e}"),
    }

    // Reminder channels at different importance levels
    // (Android won't let you modify a channel after creation, so we create all three)
    let silent = Channel::builder(REMINDER_CHANNEL_SILENT, "Reminders — Silent")
        .description("Silent reminders (no sound or vibration)".to_string())
        .importance(Importance::Low)
        .vibration(false)
        .build();

    let normal = Channel::builder(REMINDER_CHANNEL_NORMAL, "Reminders — Normal")
        .description("Reminders with sound".to_string())
        .importance(Importance::Default)
        .vibration(false)
        .build();

    let urgent = Channel::builder(REMINDER_CHANNEL_URGENT, "Reminders — Urgent")
        .description("Heads-up reminders with sound and vibration".to_string())
        .importance(Importance::High)
        .vibration(true)
        .lights(true)
        .build();

    for ch in [silent, normal, urgent] {
        if let Err(e) = app.notification().create_channel(ch) {
            log::error!("[notif] Failed to create reminder channel: {e}");
        }
    }
    log::info!("[notif] All notification channels created");
}

/// Get the active reminder channel ID based on user preference.
pub fn get_reminder_channel(app: &AppHandle) -> String {
    use tauri_plugin_store::StoreExt;

    app.store("settings.json")
        .ok()
        .and_then(|s| s.get("reminder_channel"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| REMINDER_CHANNEL_URGENT.to_string())

}

// ---------------------------------------------------------------------------
// Request notification permission (Android 13+)
// ---------------------------------------------------------------------------

pub fn request_notification_permission(app: &AppHandle) {
    match app.notification().permission_state() {
        Ok(state) => {
            log::info!("[notif] Current permission state: {:?}", state);
            if state != tauri::plugin::PermissionState::Granted {
                log::info!("[notif] Requesting notification permission...");
                match app.notification().request_permission() {
                    Ok(new_state) => log::info!("[notif] Permission after request: {:?}", new_state),
                    Err(e) => log::error!("[notif] Failed to request permission: {e}"),
                }
            } else {
                log::info!("[notif] Permission already granted");
            }
        }
        Err(e) => log::error!("[notif] Failed to check permission state: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Ongoing timer notification
// ---------------------------------------------------------------------------

pub fn show_ongoing_notification(app: &AppHandle, task_name: &str, project_name: &str, elapsed_secs: u64) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let time_str = format!("{:02}:{:02}:{:02}", h, m, s);

    let title = format!("⏱ {} — {}", time_str, task_name);
    let body = if project_name.is_empty() {
        String::new()
    } else {
        project_name.to_string()
    };

    log::debug!("[notif] Showing ongoing notification: {} - {}", title, body);

    match app
        .notification()
        .builder()
        .id(ONGOING_NOTIFICATION_ID)
        .channel_id(TIMER_CHANNEL_ID)
        .title(&title)
        .body(&body)
        .extra("type", "ongoing")
        .ongoing()
        .silent()
        .show()
    {
        Ok(_) => log::debug!("[notif] Ongoing notification shown OK"),
        Err(e) => log::error!("[notif] Failed to show ongoing notification: {e}"),
    }
}

pub fn remove_ongoing_notification(app: &AppHandle) {
    log::info!("[notif] Removing ongoing notification");
    if let Err(e) = app.notification().remove_active(vec![ONGOING_NOTIFICATION_ID]) {
        log::error!("[notif] Failed to remove ongoing notification: {e}");
    }
}

// ---------------------------------------------------------------------------
// Reminder notification with action buttons
// ---------------------------------------------------------------------------

pub fn show_reminder_notification(
    app: &AppHandle,
    task_name: &str,
    project_name: &str,
    elapsed_secs: u64,
) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let time_str = if h > 0 {
        format!("{}h {:02}m", h, m)
    } else {
        format!("{}m", m)
    };

    let title = format!("Still working? — {}", time_str);
    let body = if project_name.is_empty() {
        task_name.to_string()
    } else {
        format!("{} · {}", task_name, project_name)
    };

    let channel = get_reminder_channel(app);
    log::info!("[notif] Showing reminder notification on channel '{}': {} - {}", channel, title, body);

    match app
        .notification()
        .builder()
        .id(REMINDER_NOTIFICATION_ID)
        .channel_id(&channel)
        .title(&title)
        .body(&body)
        .extra("type", "reminder")
        .action_type_id(REMINDER_ACTION_TYPE)
        .auto_cancel()
        .show()
    {
        Ok(_) => log::info!("[notif] Reminder notification shown OK"),
        Err(e) => log::error!("[notif] Failed to show reminder notification: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Background loop: updates the ongoing notification every 30s
// ---------------------------------------------------------------------------

const ONGOING_TICK_SECS: u64 = 30;

pub async fn run_ongoing_notification_loop(app_handle: AppHandle) {
    log::info!("[notif] Ongoing notification loop started, waiting 3s...");
    // Small initial delay so the app finishes setup
    time::sleep(Duration::from_secs(3)).await;
    log::info!("[notif] Ongoing notification loop entering main loop");

    loop {
        time::sleep(Duration::from_secs(ONGOING_TICK_SECS)).await;

        let state = app_handle.state::<AppState>();
        let timer_info = {
            let timer = state.timer.lock().unwrap();
            timer.get_state()
        };

        if timer_info.is_running {
            log::debug!(
                "[notif] Ongoing tick: timer running, elapsed={}s, task={:?}",
                timer_info.elapsed_secs,
                timer_info.task_name
            );
            show_ongoing_notification(
                &app_handle,
                &timer_info.task_name.unwrap_or_default(),
                &timer_info.project_name.unwrap_or_default(),
                timer_info.elapsed_secs,
            );
        }
        // Don't remove here — removal happens on explicit stop/discard events
    }
}
