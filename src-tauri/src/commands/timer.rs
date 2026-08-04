use crate::db::session::OutboxAction;
use crate::db::tasks::touch_recent;
use crate::devicesync::{self, DeviceIdentity, PendingLog};
use crate::error::AppResult;
use crate::state::AppState;
use crate::timer::engine::{TimerResult, TimerStateInfo};
use crate::timer::persistence::{clear_timer_state, save_timer_state};

#[tauri::command]
pub async fn start_timer(
    task_id: i64,
    task_name: String,
    project_id: i64,
    project_name: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!(
        "Starting timer for task {} '{}' (project: '{}')",
        task_id,
        task_name,
        project_name
    );

    // Minted before the locks: the session is local metadata, publishing it to
    // Odoo is the reconciler's job and must not delay the click.
    let session = devicesync::new_session(&app);

    {
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();

        timer.start(
            task_id,
            task_name.clone(),
            project_id,
            project_name.clone(),
            session,
        )?;
        save_timer_state(&db, &timer)?;
        touch_recent(&db, task_id, &task_name, Some(project_name.as_str()))?;

        // Reset the reminder elapsed counter so the user gets a full interval on the new task
        let mut reminder = state.reminder.lock().unwrap();
        reminder.reset_elapsed = true;
    }

    // Any log form still open for the previous run is moot now.
    *state.pending_log.lock().unwrap() = None;

    // A new run is worth pushing to the other devices right away.
    devicesync::nudge(&app);

    // Mobile: show ongoing notification and schedule backup reminder
    #[cfg(mobile)]
    {
        log::info!("[timer] Mobile: showing ongoing notification for '{}'", task_name);
        crate::notification::show_ongoing_notification(&app, &task_name, &project_name, 0);
        let sched_app = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::reminder::schedule_next_reminder(&sched_app).await;
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_timer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<TimerResult> {
    log::info!("Stopping timer");

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let result = {
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();

        let result = timer.stop()?;
        clear_timer_state(&db)?;

        // Hand the Odoo side to the outbox. From here the reconciler owns it:
        // it strips the live marker and writes the elapsed hours, so the entry
        // is correct even if the user never submits the log form. A later
        // `log_time` for this session amends that same line.
        //
        // A run with no elapsed time has nothing to record — the callers that
        // stop-and-log skip it too — so drop its live line rather than leaving
        // a 0h entry behind in Odoo.
        let empty = result.elapsed_secs == 0;
        let action = if empty {
            OutboxAction::Discard
        } else {
            OutboxAction::Finalize
        };
        let queued = devicesync::enqueue_finish(&db, &result, action, &result.task_name, &date);

        *state.pending_log.lock().unwrap() = if queued && !empty {
            Some(PendingLog {
                session_id: result.session_id.clone(),
                task_id: result.task_id,
                odoo_line_id: result.odoo_line_id,
                stopped_at: chrono::Utc::now(),
            })
        } else {
            None
        };
        result
    };

    devicesync::nudge(&app);

    log::info!(
        "Timer stopped: task {} '{}', elapsed {}s",
        result.task_id,
        result.task_name,
        result.elapsed_secs
    );

    // Mobile: remove ongoing notification and cancel scheduled reminder
    #[cfg(mobile)]
    {
        crate::notification::remove_ongoing_notification(&app);
        crate::reminder::cancel_scheduled_reminder(&app);
    }
    Ok(result)
}

#[tauri::command]
pub async fn discard_timer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!("Discarding timer");

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    {
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();

        // `stop` rather than `discard`: the elapsed time is thrown away all the
        // same, but the result carries the live line that has to be removed
        // from Odoo. Same "no timer is running" error when idle.
        let result = timer.stop()?;
        clear_timer_state(&db)?;
        devicesync::enqueue_finish(&db, &result, OutboxAction::Discard, "", &date);
        *state.pending_log.lock().unwrap() = None;
    }

    devicesync::nudge(&app);

    // Mobile: remove ongoing notification and cancel scheduled reminder
    #[cfg(mobile)]
    {
        crate::notification::remove_ongoing_notification(&app);
        crate::reminder::cancel_scheduled_reminder(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_timer_state(state: tauri::State<'_, AppState>) -> AppResult<TimerStateInfo> {
    let timer = state.timer.lock().unwrap();
    Ok(timer.get_state())
}

/// This install's sync identity, so the UI can tell "started here" from
/// "started on another device" when reading a timer's `origin_device`.
#[tauri::command]
pub async fn get_device_identity(state: tauri::State<'_, AppState>) -> AppResult<DeviceIdentity> {
    Ok(state.device.lock().unwrap().clone())
}

/// Reconcile with Odoo right now — used when the window comes back to the
/// foreground, where waiting out the poll interval would feel stale.
#[tauri::command]
pub async fn sync_devices_now(app: tauri::AppHandle) -> AppResult<()> {
    devicesync::nudge(&app);
    Ok(())
}
