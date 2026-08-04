use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Manager,
};

use crate::icon::generate_tray_icon;
use crate::odoo::attendance::AttendanceStatus;
use crate::state::AppState;

const TRAY_ID: &str = "main-tray";

/// Initial tray setup. Builds a default menu (not checked in) and registers event handlers.
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let status_item = MenuItemBuilder::with_id("attendance_status", "○ Not checked in")
        .enabled(false)
        .build(app)?;
    let toggle_item =
        MenuItemBuilder::with_id("attendance_toggle", "Check In").build(app)?;
    let open_item = MenuItemBuilder::with_id("open", "Open").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&status_item)
        .item(&toggle_item)
        .separator()
        .item(&open_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let (w, h, rgba) = generate_tray_icon(64);
    let icon = Image::new_owned(rgba, w, h);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Pointeuse")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            "attendance_toggle" => {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    handle_attendance_toggle(&app_handle).await;
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Rebuild the tray menu to reflect the current attendance status.
pub fn rebuild_tray(
    app: &AppHandle,
    status: &AttendanceStatus,
) -> tauri::Result<()> {
    let (status_label, toggle_label, tooltip) = if status.is_checked_in {
        let check_in_display = status
            .check_in_time
            .as_deref()
            .and_then(|t| {
                // Extract HH:MM from "YYYY-MM-DD HH:MM:SS"
                t.split(' ').nth(1).map(|time_part| {
                    time_part
                        .split(':')
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(":")
                })
            })
            .unwrap_or_default();

        (
            format!("● Checked in (since {check_in_display})"),
            "Check Out".to_string(),
            format!("Time Tracker - Checked in since {check_in_display}"),
        )
    } else {
        (
            "○ Not checked in".to_string(),
            "Check In".to_string(),
            "Time Tracker".to_string(),
        )
    };

    let status_item = MenuItemBuilder::with_id("attendance_status", &status_label)
        .enabled(false)
        .build(app)?;
    let toggle_item =
        MenuItemBuilder::with_id("attendance_toggle", &toggle_label).build(app)?;
    let open_item = MenuItemBuilder::with_id("open", "Open").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&status_item)
        .item(&toggle_item)
        .separator()
        .item(&open_item)
        .separator()
        .item(&quit_item)
        .build()?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_menu(Some(menu));
        let _ = tray.set_tooltip(Some(&tooltip));
    }

    Ok(())
}

/// Handle the attendance toggle from the tray menu.
async fn handle_attendance_toggle(app: &AppHandle) {
    let client = {
        let state = app.state::<AppState>();
        let odoo_guard = state.odoo.lock().unwrap();
        match odoo_guard.as_ref() {
            Some(c) => c.clone(),
            None => {
                log::warn!("Attendance toggle: not logged in");
                return;
            }
        }
    };

    // Get current status
    let status = match client.get_attendance_status().await {
        Ok(s) => s,
        Err(e) => {
            log::error!("Attendance toggle: failed to get status: {e}");
            return;
        }
    };

    if status.is_checked_in {
        // Auto-stop running timer on check-out
        crate::devicesync::auto_stop_timer(app, &client).await;

        match client.check_out().await {
            Ok(hours) => {
                log::info!("Tray: checked out, worked {hours:.2}h today");
                let new_status = AttendanceStatus {
                    is_checked_in: false,
                    attendance_id: None,
                    check_in_time: None,
                };
                if let Err(e) = rebuild_tray(app, &new_status) {
                    log::error!("Failed to rebuild tray after check-out: {e}");
                }
            }
            Err(e) => {
                log::error!("Tray: check-out failed: {e}");
            }
        }
    } else {
        match client.check_in().await {
            Ok((id, time)) => {
                log::info!("Tray: checked in, attendance_id={id}");
                let new_status = AttendanceStatus {
                    is_checked_in: true,
                    attendance_id: Some(id),
                    check_in_time: Some(time),
                };
                if let Err(e) = rebuild_tray(app, &new_status) {
                    log::error!("Failed to rebuild tray after check-in: {e}");
                }
            }
            Err(e) => {
                log::error!("Tray: check-in failed: {e}");
            }
        }
    }
}
