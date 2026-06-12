use crate::db::tasks::touch_recent;
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

    let db = state.db.lock().unwrap();
    let mut timer = state.timer.lock().unwrap();

    timer.start(task_id, task_name.clone(), project_id, project_name.clone())?;
    save_timer_state(&db, &timer)?;
    touch_recent(&db, task_id, &task_name, Some(project_name.as_str()))?;

    // Reset the reminder elapsed counter so the user gets a full interval on the new task
    {
        let mut reminder = state.reminder.lock().unwrap();
        reminder.reset_elapsed = true;
    }

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
    #[cfg(desktop)]
    let _ = &app; // suppress unused warning

    Ok(())
}

#[tauri::command]
pub async fn stop_timer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<TimerResult> {
    log::info!("Stopping timer");

    let db = state.db.lock().unwrap();
    let mut timer = state.timer.lock().unwrap();

    let result = timer.stop()?;
    clear_timer_state(&db)?;

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
    #[cfg(desktop)]
    let _ = &app;

    Ok(result)
}

#[tauri::command]
pub async fn discard_timer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!("Discarding timer");

    let db = state.db.lock().unwrap();
    let mut timer = state.timer.lock().unwrap();

    timer.discard()?;
    clear_timer_state(&db)?;

    // Mobile: remove ongoing notification and cancel scheduled reminder
    #[cfg(mobile)]
    {
        crate::notification::remove_ongoing_notification(&app);
        crate::reminder::cancel_scheduled_reminder(&app);
    }
    #[cfg(desktop)]
    let _ = &app;

    Ok(())
}

#[tauri::command]
pub async fn get_timer_state(state: tauri::State<'_, AppState>) -> AppResult<TimerStateInfo> {
    let timer = state.timer.lock().unwrap();
    Ok(timer.get_state())
}
