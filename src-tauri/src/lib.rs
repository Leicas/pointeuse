mod commands;
mod credentials;
mod db;
mod error;
#[cfg(desktop)]
mod icon;
#[cfg(mobile)]
mod notification;
mod odoo;
mod reminder;
mod state;
mod timer;
#[cfg(desktop)]
mod tray;

use std::sync::Mutex;

use chrono::Datelike;
use tauri::Manager;
use tauri_plugin_store::StoreExt;

use tauri::Emitter;

use db::schema::initialize_database;
use reminder::ReminderState;
use state::AppState;
use timer::engine::TimerEngine;
use timer::persistence::restore_timer_state;

/// Background loop that pre-fetches and caches timesheet/attendance data
/// for the current week. Runs every 5 minutes after login.
async fn run_cache_sync(app_handle: tauri::AppHandle) {
    use tokio::time::{self, Duration};

    // Wait for auto-login to complete
    time::sleep(Duration::from_secs(15)).await;

    loop {
        // Check if logged in
        let client = {
            let state = app_handle.state::<AppState>();
            let odoo_guard = state.odoo.lock().unwrap();
            odoo_guard.as_ref().cloned()
        };

        if let Some(client) = client {
            let _ = app_handle.emit("cache_sync_start", "weekly_sync");
            let today = chrono::Local::now();
            let today_str = today.format("%Y-%m-%d").to_string();

            // Pre-fetch current week: Monday through Sunday
            let weekday = today.weekday().num_days_from_monday(); // 0=Mon
            let monday = today - chrono::Duration::days(weekday as i64);

            for i in 0..7i64 {
                let day = monday + chrono::Duration::days(i);
                let date_str = day.format("%Y-%m-%d").to_string();

                // Check if this date's cache is fresh enough
                let needs_refresh = {
                    let state = app_handle.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let age = db::cache::get_cache_age_secs(&db, &format!("ts:{date_str}")).unwrap_or(None);
                    let max_age = if date_str == today_str { 60 } else { 300 };
                    age.map_or(true, |a| a > max_age)
                };

                if !needs_refresh {
                    continue;
                }

                // Fetch timesheets for this day
                if let Ok(raw) = client.get_today_timesheets(&date_str).await {
                    let entries = commands::timesheet::convert_odoo_entries(raw);
                    let state = app_handle.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = db::cache::cache_timesheet_entries(&db, &date_str, &entries);
                    log::debug!("cache_sync: cached {} entries for {date_str}", entries.len());
                }

                // Also cache attendance for today (most useful for analysis)
                if date_str == today_str {
                    if let Ok(att_raw) = client.get_today_attendance(&date_str).await {
                        let blocks: Vec<commands::analysis::AttendanceBlock> = att_raw
                            .into_iter()
                            .map(|(check_in, check_out, worked_hours)| {
                                commands::analysis::AttendanceBlock { check_in, check_out, worked_hours }
                            })
                            .collect();
                        let state = app_handle.state::<AppState>();
                        let db = state.db.lock().unwrap();
                        let _ = db::cache::cache_attendance(&db, &date_str, &blocks);
                    }
                }

                // Small delay between fetches to not hammer Odoo
                time::sleep(Duration::from_millis(200)).await;
            }

            let _ = app_handle.emit("cache_sync_done", "weekly_sync");
        }

        // Run every 5 minutes
        time::sleep(Duration::from_secs(300)).await;
    }
}

/// Background loop that polls Odoo attendance every 60 seconds.
/// Detects changes and notifies frontend + rebuilds tray.
async fn run_attendance_poll(app_handle: tauri::AppHandle) {
    use tokio::time::{self, Duration};

    // Wait for auto-login to complete
    time::sleep(Duration::from_secs(10)).await;

    loop {
        time::sleep(Duration::from_secs(60)).await;

        // Check if logged in
        let client = {
            let state = app_handle.state::<AppState>();
            let odoo_guard = state.odoo.lock().unwrap();
            odoo_guard.as_ref().cloned()
        };

        let client = match client {
            Some(c) => c,
            None => continue,
        };

        // Fetch attendance from Odoo
        let status = match client.get_attendance_status().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Check if changed
        let changed = {
            let state = app_handle.state::<AppState>();
            let mut last = state.last_attendance.lock().unwrap();
            let changed = last.as_ref() != Some(&status);
            if changed {
                *last = Some(status.clone());
            }
            changed
        };

        if changed {
            log::info!(
                "Attendance changed: checked_in={}",
                status.is_checked_in
            );

            // If user just checked out externally, auto-stop running timer
            if !status.is_checked_in {
                let stopped_result = {
                    let state = app_handle.state::<AppState>();
                    let mut timer_guard = state.timer.lock().unwrap();
                    if timer_guard.is_running() {
                        match timer_guard.stop() {
                            Ok(result) => {
                                let db = state.db.lock().unwrap();
                                let _ = timer::persistence::clear_timer_state(&db);
                                Some(result)
                            }
                            Err(e) => {
                                log::error!("Attendance poll: failed to stop timer: {e}");
                                None
                            }
                        }
                    } else {
                        None
                    }
                };

                if let Some(result) = stopped_result {
                    let hours = result.elapsed_secs as f64 / 3600.0;
                    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
                    commands::timesheet::log_time_with_fallback(
                        &app_handle, &client,
                        result.task_id, result.project_id,
                        &result.task_name, &result.project_name,
                        hours, &date,
                    ).await;
                    log::info!(
                        "Attendance poll: auto-stopped timer for '{}' ({:.2}h) on external checkout",
                        result.task_name, hours
                    );
                    let _ = app_handle.emit("timer_auto_stopped", &result);
                }

                // Dismiss any open reminder popup (desktop window + in-app overlay)
                {
                    let state = app_handle.state::<AppState>();
                    let mut reminder = state.reminder.lock().unwrap();
                    reminder.popup_showing = false;
                    reminder.reset_elapsed = true;
                }
                #[cfg(desktop)]
                if let Some(win) = app_handle.get_webview_window("reminder") {
                    let _ = win.close();
                    log::info!("Attendance poll: closed reminder popup on external checkout");
                }
                let _ = app_handle.emit("dismiss_reminder", ());
            }

            // Rebuild tray (desktop only)
            #[cfg(desktop)]
            if let Err(e) = tray::rebuild_tray(&app_handle, &status) {
                log::error!("Failed to rebuild tray on attendance change: {e}");
            }
            // Notify frontend
            let _ = app_handle.emit("attendance_changed", &status);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // The single-instance plugin must be registered FIRST (per Tauri guidance):
    // a second launch hands its args to this callback and then exits, so we
    // surface the already-running window instead of opening a duplicate app.
    // Desktop-only — the plugin has no mobile implementation.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));

    let builder = builder
        // schedule-task must be initialized first (required by plugin for desktop scheduling)
        .plugin(tauri_plugin_schedule_task::init_with_handler(
            reminder::ScheduledTaskRouter,
        ))
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        );

    // Desktop-only plugins
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    builder
        .setup(|app| {
            // Initialize database
            let app_data_dir = app.path().app_data_dir()?;
            let conn = initialize_database(&app_data_dir)?;

            // Run incremental migrations
            timer::persistence::ensure_project_id_column(&conn);

            // Initialize timer engine, restoring from DB if crashed
            let mut timer_engine = TimerEngine::new();
            if let Ok(Some(saved)) = restore_timer_state(&conn) {
                log::info!(
                    "Restoring timer for task {} '{}'",
                    saved.task_id,
                    saved.task_name
                );
                timer_engine.restore(
                    saved.task_id,
                    saved.task_name,
                    saved.project_id,
                    saved.project_name,
                    saved.start_utc,
                    saved.accumulated_secs,
                );
            }

            // Load reminder interval from store
            let reminder_interval = app
                .store("settings.json")
                .ok()
                .and_then(|s| s.get("reminder_interval_minutes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            // Create app state
            let state = AppState {
                db: Mutex::new(conn),
                odoo: Mutex::new(None),
                timer: Mutex::new(timer_engine),
                reminder: Mutex::new(ReminderState {
                    interval_minutes: reminder_interval,
                    popup_showing: false,
                    scheduled_task_id: None,
                    reset_elapsed: false,
                }),
                last_attendance: Mutex::new(None),
                tasks_cache: Mutex::new(None),
                projects_cache: Mutex::new(None),
                sync_in_progress: Mutex::new(false),
                #[cfg(desktop)]
                pending_update: Mutex::new(None),
            };
            app.manage(state);

            // Desktop: set window icon and setup system tray
            #[cfg(desktop)]
            {
                // Set the window icon from the embedded PNG
                // (ensures correct icon in both dev and prod, regardless of exe resource)
                if let Some(window) = app.get_webview_window("main") {
                    let icon = tauri::image::Image::from_bytes(
                        include_bytes!("../icons/icon.png"),
                    )
                    .or_else(|_| {
                        // Fallback: try the .ico
                        tauri::image::Image::from_bytes(
                            include_bytes!("../icons/icon.ico"),
                        )
                    });
                    match icon {
                        Ok(img) => { let _ = window.set_icon(img); }
                        Err(e) => log::warn!("Failed to set app icon: {e}"),
                    }
                }

                tray::setup_tray(app.handle())?;
            }

            // Mobile: setup notification channels and request permission
            #[cfg(mobile)]
            {
                log::info!("[setup] Mobile: setting up notification channels...");
                notification::setup_notification_channels(app.handle());
                log::info!("[setup] Mobile: requesting notification permission...");
                notification::request_notification_permission(app.handle());
            }

            // Spawn idle-reminder background loop (both desktop and mobile)
            {
                let reminder_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    reminder::run_reminder_loop(reminder_handle).await;
                });
            }

            // Mobile: spawn ongoing notification update loop
            #[cfg(mobile)]
            {
                log::info!("[setup] Mobile: spawning ongoing notification loop");
                let notif_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    notification::run_ongoing_notification_loop(notif_handle).await;
                });
            }

            // Spawn attendance polling loop (tokio — works while app is alive)
            let attendance_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_attendance_poll(attendance_handle).await;
            });

            // Mobile: schedule background attendance check via WorkManager
            #[cfg(mobile)]
            {
                let attendance_sched = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    reminder::schedule_attendance_check(&attendance_sched).await;
                });
            }

            // Spawn cache sync loop (pre-fetches current week data)
            let cache_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_cache_sync(cache_handle).await;
            });

            // Spawn auto-login and tray rebuild
            let auto_login_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Try auto-login
                if commands::auth::try_auto_login(&auto_login_handle).await.is_some() {
                    // Fetch attendance status and rebuild tray
                    let client = {
                        let state = auto_login_handle.state::<AppState>();
                        let odoo_guard = state.odoo.lock().unwrap();
                        odoo_guard.as_ref().cloned()
                    };
                    if let Some(client) = client {
                        match client.get_attendance_status().await {
                            Ok(_status) => {
                                #[cfg(desktop)]
                                if let Err(e) = tray::rebuild_tray(&auto_login_handle, &_status) {
                                    log::error!("Failed to rebuild tray after auto-login: {e}");
                                }
                            }
                            Err(e) => {
                                log::warn!("Could not fetch attendance status after auto-login: {e}");
                            }
                        }
                    }
                }
            });

            // Desktop: intercept close to minimize to tray instead
            #[cfg(desktop)]
            {
                let window = app.get_webview_window("main").unwrap();
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::auth::logout,
            commands::auth::check_auth,
            commands::auth::detect_database,
            commands::auth::get_saved_connection,
            commands::timer::start_timer,
            commands::timer::stop_timer,
            commands::timer::discard_timer,
            commands::timer::get_timer_state,
            commands::tasks::search_tasks,
            commands::tasks::get_my_tasks,
            commands::tasks::get_recent_tasks,
            commands::tasks::create_task,
            commands::tasks::get_suggested_tasks,
            commands::tasks::get_task_stages,
            commands::tasks::update_task_stage,
            commands::tasks::update_task_kanban_state,
            commands::tasks::update_task_state,
            commands::tasks::update_task_name,
            commands::tasks::update_task_description,
            commands::tasks::update_task_deadline,
            commands::tasks::update_task_priority,
            commands::tasks::get_task_details,
            commands::tasks::get_all_tasks,
            commands::tasks::get_all_users,
            commands::timesheet::log_time,
            commands::timesheet::get_today_entries,
            commands::projects::get_projects,
            commands::sync::sync_pending,
            commands::sync::get_sync_status,
            commands::sync::get_pending_entries,
            commands::sync::get_review_entries,
            commands::sync::resolve_sync_entry,
            commands::sync::retry_sync_entry,
            commands::attendance::get_attendance_status,
            commands::attendance::attendance_check_in,
            commands::attendance::attendance_check_out,
            commands::reminder::set_reminder_interval,
            commands::reminder::get_reminder_interval,
            commands::reminder::dismiss_idle_reminder,
            commands::reminder::test_reminder_popup,
            commands::reminder::get_reminder_channel,
            commands::reminder::set_reminder_channel,
            commands::autostart::get_autostart_enabled,
            commands::autostart::set_autostart_enabled,
            commands::analysis::get_day_analysis,
            commands::timesheet::get_entries_for_date,
            commands::timesheet::get_monthly_summary,
            commands::timesheet::preflight_manual_entry,
            commands::timesheet::create_manual_entry,
            commands::timesheet::update_timesheet_entry,
            commands::timesheet::delete_timesheet_entry,
            commands::timesheet::update_pending_entry,
            commands::timesheet::get_pending_for_date,
            commands::updater::check_for_update,
            commands::updater::install_update,
            commands::settings::get_quickswitch_mode,
            commands::settings::set_quickswitch_mode,
            commands::settings::get_quickswitch_items,
            commands::settings::set_quickswitch_items,
            commands::settings::get_hide_done_tasks,
            commands::settings::set_hide_done_tasks,
            commands::settings::get_quick_switch_entries,
            commands::settings::get_default_task,
            commands::settings::set_default_task,
            commands::settings::clear_default_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
