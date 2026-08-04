use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;

use tauri::{Emitter, Manager};

use crate::db::cache;
use crate::db::timesheets::{
    add_to_log, delete_log_by_odoo_id, get_log_for_date, get_monthly_log_summary, get_today_log,
    queue_timesheet, update_pending, PendingTimesheet,
};
use crate::error::{AppError, AppResult};
use crate::odoo::models::OdooTimesheetEntry;
use crate::odoo::xmlrpc::XmlRpcValue;
use crate::state::AppState;

/// Return the last day of a given year/month as "YYYY-MM-DD".
fn last_day_of_month(year: i32, month: u32) -> String {
    // First day of next month, then subtract one day
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    let last = next.unwrap().pred_opt().unwrap();
    last.format("%Y-%m-%d").to_string()
}

/// Log time with private-task fallback to default task.
/// Used by auto-stop paths (tray checkout, attendance poll) that don't go through the command.
///
/// Returns the Odoo line id when one was created, so callers that may later
/// amend the entry (the cross-device log form) know what to write to.
#[allow(clippy::too_many_arguments)]
pub async fn log_time_with_fallback(
    app: &tauri::AppHandle,
    client: &crate::odoo::client::OdooClient,
    task_id: i64,
    project_id: i64,
    task_name: &str,
    project_name: &str,
    hours: f64,
    date: &str,
) -> Option<i64> {
    use tauri::Emitter;
    use tauri_plugin_store::StoreExt;

    let default_task: Option<crate::commands::settings::DefaultTaskConfig> = app
        .store("settings.json")
        .ok()
        .and_then(|s| s.get("default_task"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    match client.log_time(task_id, project_id, task_name, hours, date).await {
        Ok(odoo_id) => {
            log::info!("log_time_with_fallback: logged {:.2}h for '{}', odoo_id={}", hours, task_name, odoo_id);
            let state = app.state::<AppState>();
            let db = state.db.lock().unwrap();
            let _ = crate::db::timesheets::add_to_log(&db, task_id, task_name, project_name, task_name, hours, date, Some(odoo_id));
            Some(odoo_id)
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.to_lowercase().contains("private task") {
                if let Some(ref dt) = default_task {
                    let desc = format!("[{}] {}", task_name, task_name);
                    log::warn!("log_time_with_fallback: private task rejected, retrying with default task '{}'", dt.task_name);
                    match client.log_time(dt.task_id, dt.project_id, &desc, hours, date).await {
                        Ok(odoo_id) => {
                            log::info!("log_time_with_fallback: redirected to default task, odoo_id={}", odoo_id);
                            let state = app.state::<AppState>();
                            let db = state.db.lock().unwrap();
                            let _ = crate::db::timesheets::add_to_log(&db, dt.task_id, &dt.task_name, &dt.project_name, &desc, hours, date, Some(odoo_id));
                            let _ = app.emit("time_redirected", serde_json::json!({
                                "original_task": task_name,
                                "default_task": dt.task_name,
                                "hours": hours,
                            }));
                            Some(odoo_id)
                        }
                        Err(e2) => {
                            log::error!("log_time_with_fallback: default task also failed, queuing: {e2}");
                            let state = app.state::<AppState>();
                            let db = state.db.lock().unwrap();
                            let _ = crate::db::timesheets::queue_timesheet(&db, dt.task_id, dt.project_id, &dt.task_name, &dt.project_name, &desc, hours, date, false);
                            let _ = crate::db::timesheets::add_to_log(&db, dt.task_id, &dt.task_name, &dt.project_name, &desc, hours, date, None);
                            None
                        }
                    }
                } else {
                    log::error!("log_time_with_fallback: private task rejected, no default task configured. Queuing: {e}");
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = crate::db::timesheets::queue_timesheet(&db, task_id, project_id, task_name, project_name, task_name, hours, date, false);
                    let _ = crate::db::timesheets::add_to_log(&db, task_id, task_name, project_name, task_name, hours, date, None);
                    None
                }
            } else {
                log::error!("log_time_with_fallback: Odoo failed, queuing: {e}");
                let state = app.state::<AppState>();
                let db = state.db.lock().unwrap();
                let _ = crate::db::timesheets::queue_timesheet(&db, task_id, project_id, task_name, project_name, task_name, hours, date, false);
                let _ = crate::db::timesheets::add_to_log(&db, task_id, task_name, project_name, task_name, hours, date, None);
                None
            }
        }
    }
}

/// Max cache age (seconds) before a background refresh is triggered.
const CACHE_MAX_AGE_TODAY: i64 = 60;      // 1 minute for today
const CACHE_MAX_AGE_PAST: i64 = 3600;     // 1 hour for past dates

#[derive(Debug, Clone, Serialize)]
pub struct LogTimeResult {
    pub success: bool,
    pub odoo_id: Option<i64>,
    pub queued: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimesheetEntry {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub task_name: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub description: String,
    pub hours: f64,
    pub date: String,
    pub source: String, // "odoo", "local", or "cache"
}

/// Convert Odoo entries to our TimesheetEntry format.
pub fn convert_odoo_entries(entries: Vec<OdooTimesheetEntry>) -> Vec<TimesheetEntry> {
    entries
        .into_iter()
        .map(|e| {
            let (task_id, task_name) = match e.task_id {
                Some((id, name)) => (Some(id), name),
                None => (None, String::new()),
            };
            let (project_id, project_name) = match e.project_id {
                Some((id, name)) => (Some(id), name),
                None => (None, String::new()),
            };
            TimesheetEntry {
                id: Some(e.id),
                task_id,
                task_name,
                project_id,
                project_name,
                description: e.name,
                hours: e.unit_amount,
                date: e.date,
                source: "odoo".into(),
            }
        })
        .collect()
}

/// Determine the max cache age for a given date.
fn max_age_for_date(date: &str) -> i64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    if date == today { CACHE_MAX_AGE_TODAY } else { CACHE_MAX_AGE_PAST }
}

/// Spawn a background task to refresh timesheet entries for a date from Odoo,
/// save to cache, and emit an event to the frontend.
fn spawn_entries_refresh(app: tauri::AppHandle, date: String) {
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("cache_sync_start", format!("entries:{date}"));
        let state: tauri::State<'_, AppState> = app.state::<AppState>();
        let client: Option<crate::odoo::client::OdooClient> = {
            let odoo = state.odoo.lock().unwrap();
            odoo.clone()
        };
        let Some(client) = client else {
            let _ = app.emit("cache_sync_done", format!("entries:{date}"));
            return;
        };

        match client.get_today_timesheets(&date).await {
            Ok(raw) => {
                let entries = convert_odoo_entries(raw);
                // Save to cache
                {
                    let db = state.db.lock().unwrap();
                    let _ = cache::cache_timesheet_entries(&db, &date, &entries);
                }
                log::info!("cache: refreshed {} entries for {date}", entries.len());
                let _ = app.emit("entries_refreshed", serde_json::json!({
                    "date": date,
                    "entries": entries,
                }));
            }
            Err(e) => {
                log::warn!("cache: background refresh failed for {date}: {e}");
            }
        }
        let _ = app.emit("cache_sync_done", format!("entries:{date}"));
    });
}

/// Spawn a background task to refresh a monthly range from Odoo.
fn spawn_monthly_refresh(app: tauri::AppHandle, year: i32, month: u32) {
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("cache_sync_start", format!("monthly:{year}-{month:02}"));
        let state: tauri::State<'_, AppState> = app.state::<AppState>();
        let client: Option<crate::odoo::client::OdooClient> = {
            let odoo = state.odoo.lock().unwrap();
            odoo.clone()
        };
        let Some(client) = client else {
            let _ = app.emit("cache_sync_done", format!("monthly:{year}-{month:02}"));
            return;
        };

        let start = format!("{year:04}-{month:02}-01");
        let end = last_day_of_month(year, month);

        match client.get_timesheets_for_range(&start, &end).await {
            Ok(raw) => {
                let entries = convert_odoo_entries(raw);
                // Save to cache (all dates in range)
                {
                    let db = state.db.lock().unwrap();
                    let _ = cache::cache_timesheet_entries_range(&db, &entries);
                }
                // Build summary from fresh data
                let summary = build_monthly_summary(year, month, &entries);
                log::info!("cache: refreshed monthly {year}-{month:02} ({} days)", summary.days.len());
                let _ = app.emit("monthly_refreshed", &summary);
            }
            Err(e) => {
                log::warn!("cache: monthly refresh failed for {year}-{month:02}: {e}");
            }
        }
        let _ = app.emit("cache_sync_done", format!("monthly:{year}-{month:02}"));
    });
}

/// Finish off a run that was tracked across devices.
///
/// Such a run already owns — or is queued to own — a single timesheet line in
/// Odoo, so the log form must amend that line instead of creating a second one.
/// Returns `None` when this call has nothing to do with a synced run, in which
/// case the caller falls through to the ordinary create path.
#[allow(clippy::too_many_arguments)]
async fn settle_pending_session(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    client: Option<&crate::odoo::client::OdooClient>,
    task_id: i64,
    task_name: &str,
    project_name: &str,
    description: &str,
    duration_hours: f64,
    date: &str,
) -> AppResult<Option<LogTimeResult>> {
    let pending = { state.pending_log.lock().unwrap().clone() };
    let Some(pending) = pending else {
        return Ok(None);
    };
    // The log form was retargeted at a different task — treat it as a new entry
    // and leave the session's own line to the reconciler.
    if pending.task_id != task_id {
        return Ok(None);
    }
    // Long-abandoned: the reconciler settled that run ages ago, and this call
    // is a genuinely new entry that happens to share its task.
    let age_mins = chrono::Utc::now()
        .signed_duration_since(pending.stopped_at)
        .num_minutes();
    if age_mins >= crate::devicesync::PENDING_LOG_TTL_MINS {
        *state.pending_log.lock().unwrap() = None;
        return Ok(None);
    }
    // One-shot: whatever happens below, this session is settled.
    *state.pending_log.lock().unwrap() = None;

    let lock = state.sync_lock.clone();
    let _guard = lock.lock().await;

    // Still queued: fold the user's edits into the queued write.
    let queued_entry = {
        let db = state.db.lock().unwrap();
        let amended = crate::db::session::amend_finalize(
            &db,
            &pending.session_id,
            description,
            duration_hours,
            date,
        )
        .unwrap_or(false);
        if amended {
            crate::db::session::get(&db, &pending.session_id).unwrap_or(None)
        } else {
            None
        }
    };

    let Some(client) = client else {
        // Offline. The queued row (or the pending-timesheet fallback behind it)
        // already carries the work, so nothing is lost.
        log::info!("log_time: offline, session {} stays queued", pending.session_id);
        return Ok(Some(LogTimeResult { success: false, odoo_id: None, queued: true }));
    };

    if let Some(entry) = queued_entry {
        return Ok(Some(
            match crate::devicesync::flush_one(app, client, &entry).await {
                crate::devicesync::Flushed::Done(line) => {
                    let db = state.db.lock().unwrap();
                    let _ = crate::db::session::remove(&db, &pending.session_id);
                    // A `None` line means it went to the offline queue instead.
                    LogTimeResult {
                        success: line.is_some(),
                        odoo_id: line,
                        queued: line.is_none(),
                    }
                }
                // Row stays queued; the reconciler retries with the user's edits.
                crate::devicesync::Flushed::Retry => LogTimeResult {
                    success: false,
                    odoo_id: None,
                    queued: true,
                },
            },
        ));
    }

    // Already flushed by the reconciler — rewrite the line it landed on.
    let Some(line_id) = pending.odoo_line_id else {
        log::warn!(
            "log_time: session {} was settled but its line is unknown; not creating a duplicate",
            pending.session_id
        );
        return Ok(Some(LogTimeResult { success: true, odoo_id: None, queued: false }));
    };

    let mut values = std::collections::HashMap::new();
    values.insert("name".to_string(), XmlRpcValue::String(description.to_string()));
    values.insert("unit_amount".to_string(), XmlRpcValue::Double(duration_hours));
    values.insert("date".to_string(), XmlRpcValue::String(date.to_string()));

    match client.update_timesheet_line(line_id, values).await {
        Ok(_) => {
            client.recompute_task(task_id).await;
            let db = state.db.lock().unwrap();
            let _ = delete_log_by_odoo_id(&db, line_id);
            add_to_log(&db, task_id, task_name, project_name, description, duration_hours, date, Some(line_id))?;
            Ok(Some(LogTimeResult { success: true, odoo_id: Some(line_id), queued: false }))
        }
        Err(e) => {
            log::error!("log_time: could not amend line {line_id}: {e}");
            // The line still holds the provisional description and the right
            // hours, so the time is recorded — only the edit was lost.
            Ok(Some(LogTimeResult { success: false, odoo_id: Some(line_id), queued: false }))
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn log_time(
    task_id: i64,
    project_id: i64,
    task_name: String,
    project_name: String,
    description: String,
    duration_hours: f64,
    date: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<LogTimeResult> {
    log::info!(
        "log_time: {:.2}h for task {} '{}' on {}",
        duration_hours, task_id, task_name, date
    );

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(result) = settle_pending_session(
        &app,
        &state,
        odoo_client.as_ref(),
        task_id,
        &task_name,
        &project_name,
        &description,
        duration_hours,
        &date,
    )
    .await?
    {
        spawn_entries_refresh(app, date);
        return Ok(result);
    }

    // Load default task config for private-task fallback
    let default_task: Option<crate::commands::settings::DefaultTaskConfig> = {
        use tauri_plugin_store::StoreExt;
        app.store("settings.json")
            .ok()
            .and_then(|s| s.get("default_task"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    };

    let (odoo_id, queued) = if let Some(client) = odoo_client {
        match client
            .log_time(task_id, project_id, &description, duration_hours, &date)
            .await
        {
            Ok(id) => {
                log::info!("log_time: logged to Odoo, line_id={}", id);
                (Some(id), false)
            }
            Err(e) => {
                let err_msg = e.to_string();
                // If Odoo rejects due to private task and a default task is configured,
                // retry with the default task instead of queuing a doomed entry
                if err_msg.to_lowercase().contains("private task") {
                    if let Some(ref dt) = default_task {
                        log::warn!(
                            "log_time: private task rejected, retrying with default task {} '{}'",
                            dt.task_id, dt.task_name
                        );
                        let desc = format!("[{}] {}", task_name, description);
                        match client
                            .log_time(dt.task_id, dt.project_id, &desc, duration_hours, &date)
                            .await
                        {
                            Ok(id) => {
                                log::info!("log_time: redirected to default task, line_id={}", id);
                                let _ = app.emit("time_redirected", serde_json::json!({
                                    "original_task": task_name,
                                    "default_task": dt.task_name,
                                    "hours": duration_hours,
                                }));
                                (Some(id), false)
                            }
                            Err(e2) => {
                                log::error!("log_time: default task also failed, queuing: {e2}");
                                let db = state.db.lock().unwrap();
                                queue_timesheet(&db, dt.task_id, dt.project_id, &dt.task_name, &dt.project_name, &desc, duration_hours, &date, false)?;
                                (None, true)
                            }
                        }
                    } else {
                        log::error!("log_time: private task rejected, no default task configured. Queuing (will be discarded on sync): {e}");
                        let db = state.db.lock().unwrap();
                        queue_timesheet(&db, task_id, project_id, &task_name, &project_name, &description, duration_hours, &date, false)?;
                        (None, true)
                    }
                } else {
                    log::error!("log_time: Odoo failed, queuing: {e}");
                    let db = state.db.lock().unwrap();
                    queue_timesheet(&db, task_id, project_id, &task_name, &project_name, &description, duration_hours, &date, false)?;
                    (None, true)
                }
            }
        }
    } else {
        log::info!("log_time: no Odoo connection, queuing");
        let db = state.db.lock().unwrap();
        queue_timesheet(&db, task_id, project_id, &task_name, &project_name, &description, duration_hours, &date, false)?;
        (None, true)
    };

    {
        let db = state.db.lock().unwrap();
        add_to_log(&db, task_id, &task_name, &project_name, &description, duration_hours, &date, odoo_id)?;
    }

    // Invalidate cache for this date so next read fetches fresh data
    spawn_entries_refresh(app, date);

    Ok(LogTimeResult {
        success: odoo_id.is_some(),
        odoo_id,
        queued,
    })
}

#[tauri::command]
pub async fn get_today_entries(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<TimesheetEntry>> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    log::info!("get_today_entries: fetching for {today}");

    // Check cache first
    let (cached, cache_age) = {
        let db = state.db.lock().unwrap();
        let cached = cache::get_cached_timesheet_entries(&db, &today).unwrap_or_default();
        let age = cache::get_cache_age_secs(&db, &format!("ts:{today}")).unwrap_or(None);
        (cached, age)
    };

    if !cached.is_empty() {
        // Return cached data, refresh in background if stale
        if cache_age.map_or(true, |age| age > CACHE_MAX_AGE_TODAY) {
            spawn_entries_refresh(app, today);
        }
        return Ok(cached);
    }

    // No cache — try Odoo synchronously
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.get_today_timesheets(&today).await {
            Ok(raw) => {
                let entries = convert_odoo_entries(raw);
                log::info!("get_today_entries: got {} entries from Odoo", entries.len());
                // Save to cache
                {
                    let db = state.db.lock().unwrap();
                    let _ = cache::cache_timesheet_entries(&db, &today, &entries);
                }
                return Ok(entries);
            }
            Err(e) => {
                log::error!("get_today_entries: Odoo error, falling back to local: {e}");
            }
        }
    }

    // Fallback to local log
    let db = state.db.lock().unwrap();
    let local = get_today_log(&db)?;
    log::info!("get_today_entries: returning {} local entries", local.len());
    Ok(local
        .into_iter()
        .map(|e| TimesheetEntry {
            id: e.id,
            task_id: Some(e.task_id),
            task_name: e.task_name,
            project_id: None,
            project_name: e.project_name,
            description: e.description,
            hours: e.hours,
            date: e.date,
            source: "local".into(),
        })
        .collect())
}

#[tauri::command]
pub async fn get_entries_for_date(
    date: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<TimesheetEntry>> {
    log::info!("get_entries_for_date: fetching for {date}");

    // Check cache first
    let (cached, cache_age) = {
        let db = state.db.lock().unwrap();
        let cached = cache::get_cached_timesheet_entries(&db, &date).unwrap_or_default();
        let age = cache::get_cache_age_secs(&db, &format!("ts:{date}")).unwrap_or(None);
        (cached, age)
    };

    if !cached.is_empty() {
        // Return cached data immediately — refresh in background if stale
        let max_age = max_age_for_date(&date);
        if cache_age.map_or(true, |age| age > max_age) {
            spawn_entries_refresh(app, date);
        }
        log::info!("get_entries_for_date: returning {} cached entries", cached.len());
        return Ok(cached);
    }

    // No cache — try Odoo synchronously (first time visiting this date)
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.get_today_timesheets(&date).await {
            Ok(raw) => {
                let entries = convert_odoo_entries(raw);
                log::info!("get_entries_for_date: got {} entries from Odoo", entries.len());
                // Save to cache
                {
                    let db = state.db.lock().unwrap();
                    let _ = cache::cache_timesheet_entries(&db, &date, &entries);
                }
                return Ok(entries);
            }
            Err(e) => {
                log::error!("get_entries_for_date: Odoo error, falling back to local: {e}");
            }
        }
    }

    let db = state.db.lock().unwrap();
    let local = get_log_for_date(&db, Some(&date))?;
    Ok(local
        .into_iter()
        .map(|e| TimesheetEntry {
            id: e.id,
            task_id: Some(e.task_id),
            task_name: e.task_name,
            project_id: None,
            project_name: e.project_name,
            description: e.description,
            hours: e.hours,
            date: e.date,
            source: "local".into(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct DaySummary {
    pub date: String,
    pub total_hours: f64,
    pub entry_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthlySummary {
    pub year: i32,
    pub month: u32,
    pub days: Vec<DaySummary>,
    pub total_hours: f64,
}

/// Build a MonthlySummary from a flat list of entries.
pub fn build_monthly_summary(year: i32, month: u32, entries: &[TimesheetEntry]) -> MonthlySummary {
    let mut day_map: std::collections::BTreeMap<String, (f64, u32)> =
        std::collections::BTreeMap::new();
    for e in entries {
        let entry = day_map.entry(e.date.clone()).or_insert((0.0, 0));
        entry.0 += e.hours;
        entry.1 += 1;
    }
    let days: Vec<DaySummary> = day_map
        .into_iter()
        .map(|(date, (total_hours, entry_count))| DaySummary {
            date,
            total_hours,
            entry_count,
        })
        .collect();
    let total_hours: f64 = days.iter().map(|d| d.total_hours).sum();
    MonthlySummary { year, month, days, total_hours }
}

#[tauri::command]
pub async fn get_monthly_summary(
    year: i32,
    month: u32,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<MonthlySummary> {
    log::info!("get_monthly_summary: {year}-{month:02}");

    let start = format!("{year:04}-{month:02}-01");
    let end = last_day_of_month(year, month);
    let cache_key = format!("monthly:{year}-{month:02}");

    // Try to build from cache
    let (cached_entries, cache_age) = {
        let db = state.db.lock().unwrap();
        let entries = cache::get_cached_timesheet_range(&db, &start, &end).unwrap_or_default();
        let age = cache::get_cache_age_secs(&db, &cache_key).unwrap_or(None);
        (entries, age)
    };

    if !cached_entries.is_empty() {
        let summary = build_monthly_summary(year, month, &cached_entries);
        // Refresh in background if stale
        let max_age = max_age_for_date(&start); // use first day of month
        if cache_age.map_or(true, |age| age > max_age) {
            spawn_monthly_refresh(app, year, month);
        }
        log::info!("get_monthly_summary: returning cached ({} days)", summary.days.len());
        return Ok(summary);
    }

    // No cache — try Odoo synchronously
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.get_timesheets_for_range(&start, &end).await {
            Ok(raw) => {
                let entries = convert_odoo_entries(raw);
                // Cache the entries
                {
                    let db = state.db.lock().unwrap();
                    let _ = cache::cache_timesheet_entries_range(&db, &entries);
                    // Also store the monthly cache key so we know the range was fetched
                    let _ = db.execute(
                        "INSERT OR REPLACE INTO cache_meta (cache_key, updated_at) VALUES (?1, datetime('now'))",
                        rusqlite::params![cache_key],
                    );
                }
                return Ok(build_monthly_summary(year, month, &entries));
            }
            Err(e) => {
                log::error!("get_monthly_summary: Odoo error, falling back to local: {e}");
            }
        }
    }

    // Fallback to local
    let db = state.db.lock().unwrap();
    let local = get_monthly_log_summary(&db, year, month)?;
    let days: Vec<DaySummary> = local
        .into_iter()
        .map(|(date, total_hours, entry_count)| DaySummary {
            date,
            total_hours,
            entry_count,
        })
        .collect();
    let total_hours: f64 = days.iter().map(|d| d.total_hours).sum();
    Ok(MonthlySummary {
        year,
        month,
        days,
        total_hours,
    })
}

// ── Manual timesheet entries ─────────────────────────────────────────
//
// The timer path (`log_time`) is deliberately untouched. Manual entries take a
// stricter route: they validate at the boundary, never silently redirect to the
// default task, and never write anything when Odoo refuses — the user decides
// what happens next from the composer.

/// Maximum description length accepted from the composer.
const MAX_DESCRIPTION_CHARS: usize = 512;

/// Reject an entry that is more than this many days old with a soft warning.
const VERY_OLD_DAYS: i64 = 90;

/// An `account.analytic.line` already in Odoo that looks like the entry being composed.
#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCandidate {
    pub odoo_id: i64,
    pub task_id: Option<i64>,
    pub task_name: String,
    pub description: String,
    pub hours: f64,
    pub date: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreflightWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualEntryPreflight {
    pub online: bool,
    pub day_total_hours: f64,
    pub day_entry_count: u32,
    pub duplicates: Vec<DuplicateCandidate>,
    pub timer_task_id: Option<i64>,
    pub timer_elapsed_secs: u64,
    pub warnings: Vec<PreflightWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualEntryResult {
    /// "created" | "queued" | "needs_confirm" | "rejected" | "updated"
    pub outcome: String,
    pub odoo_id: Option<i64>,
    pub pending_id: Option<i64>,
    pub error: Option<String>,
    pub is_permanent: bool,
    pub duplicates: Vec<DuplicateCandidate>,
    pub entry: Option<TimesheetEntry>,
}

impl ManualEntryResult {
    /// A result carrying nothing but an outcome tag.
    fn plain(outcome: &str) -> Self {
        Self {
            outcome: outcome.into(),
            odoo_id: None,
            pending_id: None,
            error: None,
            is_permanent: false,
            duplicates: Vec::new(),
            entry: None,
        }
    }

    /// Odoo refused the write. Nothing was persisted; the error is verbatim.
    fn rejected(err: &AppError) -> Self {
        let msg = err.to_string();
        let is_permanent = crate::commands::sync::is_permanent_error(&msg);
        Self {
            error: Some(msg),
            is_permanent,
            ..Self::plain("rejected")
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteResult {
    pub deleted: bool,
}

impl From<&OdooTimesheetEntry> for DuplicateCandidate {
    fn from(e: &OdooTimesheetEntry) -> Self {
        let (task_id, task_name) = match &e.task_id {
            Some((id, name)) => (Some(*id), name.clone()),
            None => (None, String::new()),
        };
        Self {
            odoo_id: e.id,
            task_id,
            task_name,
            description: e.name.clone(),
            hours: e.unit_amount,
            date: e.date.clone(),
        }
    }
}

/// Boundary validation shared by every manual write.
///
/// Nothing else in the app validates these: the column is a bare REAL, so a NaN
/// would poison every SUM in the aggregation path and break serialization of
/// MonthlySummary and DayAnalysis.
fn validate_manual_entry(
    task_id: i64,
    project_id: i64,
    duration_hours: f64,
    date: &str,
    description: &str,
) -> AppResult<()> {
    if !duration_hours.is_finite() {
        return Err(AppError::General("Duration is not a valid number".into()));
    }
    if duration_hours < 0.01 {
        return Err(AppError::General("Duration must be at least 0.01 h".into()));
    }
    if duration_hours > 24.0 {
        return Err(AppError::General("Duration cannot exceed 24 h".into()));
    }
    if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return Err(AppError::General(format!("Invalid date: {date}")));
    }
    if task_id <= 0 {
        return Err(AppError::General("A task must be selected".into()));
    }
    if project_id <= 0 {
        return Err(AppError::General("A project must be selected".into()));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(AppError::General(format!(
            "Description is too long (max {MAX_DESCRIPTION_CHARS} characters)"
        )));
    }
    Ok(())
}

/// Odoo stores the task name when the description is blank, matching what the
/// timer path already writes.
fn effective_description(description: &str, task_name: &str) -> String {
    if description.trim().is_empty() {
        task_name.to_string()
    } else {
        description.to_string()
    }
}

/// Tell both windows that the ledger changed, ahead of the (slower) Odoo refresh.
fn emit_ledger_changed(
    app: &tauri::AppHandle,
    date: &str,
    kind: &str,
    odoo_id: Option<i64>,
    pending_id: Option<i64>,
) {
    let _ = app.emit(
        "ledger_changed",
        serde_json::json!({
            "date": date,
            "kind": kind,
            "odoo_id": odoo_id,
            "pending_id": pending_id,
        }),
    );
}

/// Every Odoo line on `date` that would be flagged as a duplicate of this entry.
/// `exclude_odoo_id` keeps an entry being edited from matching itself.
fn collect_duplicates(
    existing: &[OdooTimesheetEntry],
    task_id: i64,
    date: &str,
    duration_hours: f64,
    exclude_odoo_id: Option<i64>,
) -> Vec<DuplicateCandidate> {
    existing
        .iter()
        .filter(|odoo| Some(odoo.id) != exclude_odoo_id)
        .filter(|odoo| {
            crate::commands::sync::is_duplicate_match(task_id, date, duration_hours, odoo)
        })
        .map(DuplicateCandidate::from)
        .collect()
}

/// Advisory checks for the composer. Read-only, never errors on bad input —
/// it reports instead, so it is safe to call on every debounced keystroke.
#[tauri::command]
pub async fn preflight_manual_entry(
    task_id: i64,
    project_id: i64,
    duration_hours: f64,
    date: String,
    exclude_odoo_id: Option<i64>,
    state: tauri::State<'_, AppState>,
) -> AppResult<ManualEntryPreflight> {
    // Accepted for symmetry with create_manual_entry; the checks below don't need it.
    let _ = project_id;

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };
    let online = odoo_client.is_some();

    // Day totals come from the same cache the day list renders from.
    // A read failure here would silently disable the DAY_OVER_16 / DAY_OVER_24
    // guardrails (an empty Vec sums to 0), so it is logged rather than swallowed.
    let (day_total_hours, day_entry_count) = {
        let db = state.db.lock().unwrap();
        let cached = cache::get_cached_timesheet_entries(&db, &date).unwrap_or_else(|e| {
            log::warn!("preflight_manual_entry: day total unavailable for {date}: {e}");
            Vec::new()
        });
        let total: f64 = cached.iter().map(|e| e.hours).sum();
        (total, cached.len() as u32)
    };

    let (timer_task_id, timer_elapsed_secs) = {
        let timer = state.timer.lock().unwrap();
        let info = timer.get_state();
        if info.is_running {
            (info.task_id, info.elapsed_secs)
        } else {
            (None, 0)
        }
    };

    let mut warnings: Vec<PreflightWarning> = Vec::new();
    let mut duplicates: Vec<DuplicateCandidate> = Vec::new();

    let checkable = task_id > 0 && duration_hours.is_finite() && duration_hours > 0.0;

    if let Some(client) = odoo_client {
        if checkable {
            match client
                .get_timesheets_for_dates(std::slice::from_ref(&date))
                .await
            {
                Ok(existing) => {
                    duplicates = collect_duplicates(
                        &existing,
                        task_id,
                        &date,
                        duration_hours,
                        exclude_odoo_id,
                    );
                }
                Err(e) => {
                    log::warn!("preflight_manual_entry: duplicate check failed for {date}: {e}");
                    warnings.push(PreflightWarning {
                        code: "DEDUP_UNAVAILABLE".into(),
                        message: "Couldn't check Odoo for duplicates right now.".into(),
                    });
                }
            }
        }
    } else {
        warnings.push(PreflightWarning {
            code: "OFFLINE_WILL_QUEUE".into(),
            message: "Offline — this entry will be queued and sent on the next sync.".into(),
        });
    }

    if let Some(first) = duplicates.first() {
        warnings.push(PreflightWarning {
            code: "DUPLICATE_LIKELY".into(),
            message: format!(
                "Odoo already has a matching line (#{}, {:.2} h) on this task and day.",
                first.odoo_id, first.hours
            ),
        });
    }

    if duration_hours.is_finite() {
        let projected = day_total_hours + duration_hours;
        if projected > 24.0 {
            warnings.push(PreflightWarning {
                code: "DAY_OVER_24".into(),
                message: format!("That would put this day at {projected:.2} h — over 24 h."),
            });
        } else if projected > 16.0 {
            warnings.push(PreflightWarning {
                code: "DAY_OVER_16".into(),
                message: format!("That would put this day at {projected:.2} h."),
            });
        }
    }

    if timer_task_id == Some(task_id) && task_id > 0 {
        warnings.push(PreflightWarning {
            code: "TIMER_RUNNING_SAME_TASK".into(),
            message: "A timer is running on this task — stopping it will log that separately."
                .into(),
        });
    }

    let today = chrono::Local::now().date_naive();
    if let Ok(parsed) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
        if parsed > today {
            warnings.push(PreflightWarning {
                code: "FUTURE_DATE".into(),
                message: "This date is in the future.".into(),
            });
        } else if (today - parsed).num_days() > VERY_OLD_DAYS {
            warnings.push(PreflightWarning {
                code: "VERY_OLD_DATE".into(),
                message: format!(
                    "This date is {} days old — timesheets may already be locked.",
                    (today - parsed).num_days()
                ),
            });
        }
    }

    Ok(ManualEntryPreflight {
        online,
        day_total_hours,
        day_entry_count,
        duplicates,
        timer_task_id,
        timer_elapsed_secs,
        warnings,
    })
}

/// Create a timesheet entry the user typed by hand.
///
/// Unlike `log_time` this never auto-redirects to the default task and never
/// queues a doomed entry behind the user's back: an Odoo refusal comes back as
/// outcome="rejected" with nothing written, and the composer decides what next.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_manual_entry(
    task_id: i64,
    project_id: i64,
    task_name: String,
    project_name: String,
    description: String,
    duration_hours: f64,
    date: String,
    allow_duplicate: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<ManualEntryResult> {
    validate_manual_entry(task_id, project_id, duration_hours, &date, &description)?;
    let description = effective_description(&description, &task_name);

    log::info!(
        "create_manual_entry: {duration_hours:.2}h for task {task_id} '{task_name}' on {date} (allow_duplicate={allow_duplicate})"
    );

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let Some(client) = odoo_client else {
        // Offline: queue it, and record the queue row's own duplicate decision.
        let pending_id = {
            let db = state.db.lock().unwrap();
            let pending_id = queue_timesheet(
                &db,
                task_id,
                project_id,
                &task_name,
                &project_name,
                &description,
                duration_hours,
                &date,
                allow_duplicate,
            )?;
            add_to_log(
                &db,
                task_id,
                &task_name,
                &project_name,
                &description,
                duration_hours,
                &date,
                None,
            )?;
            pending_id
        };
        log::info!("create_manual_entry: offline, queued as pending id={pending_id}");
        emit_ledger_changed(&app, &date, "queued", None, Some(pending_id));
        spawn_entries_refresh(app, date);
        return Ok(ManualEntryResult {
            pending_id: Some(pending_id),
            ..ManualEntryResult::plain("queued")
        });
    };

    // Duplicate gate: ask before writing, so the manual path can never produce a
    // surprise 'duplicate' row for the user to untangle later.
    if !allow_duplicate {
        match client
            .get_timesheets_for_dates(std::slice::from_ref(&date))
            .await
        {
            Ok(existing) => {
                let duplicates =
                    collect_duplicates(&existing, task_id, &date, duration_hours, None);
                if !duplicates.is_empty() {
                    log::info!(
                        "create_manual_entry: {} duplicate candidate(s), awaiting confirmation",
                        duplicates.len()
                    );
                    return Ok(ManualEntryResult {
                        duplicates,
                        ..ManualEntryResult::plain("needs_confirm")
                    });
                }
            }
            Err(e) => {
                // Same posture as the sync drain: a failed dedup fetch does not
                // block the write.
                log::warn!("create_manual_entry: duplicate check failed, proceeding: {e}");
            }
        }
    }

    match client
        .create_timesheet_line(task_id, project_id, &description, duration_hours, &date)
        .await
    {
        Ok(odoo_id) => {
            {
                let db = state.db.lock().unwrap();
                add_to_log(
                    &db,
                    task_id,
                    &task_name,
                    &project_name,
                    &description,
                    duration_hours,
                    &date,
                    Some(odoo_id),
                )?;
            }
            log::info!("create_manual_entry: created Odoo line {odoo_id}");
            let entry = TimesheetEntry {
                id: Some(odoo_id),
                task_id: Some(task_id),
                task_name,
                project_id: Some(project_id),
                project_name,
                description,
                hours: duration_hours,
                date: date.clone(),
                source: "odoo".into(),
            };
            emit_ledger_changed(&app, &date, "created", Some(odoo_id), None);
            spawn_entries_refresh(app, date);
            Ok(ManualEntryResult {
                odoo_id: Some(odoo_id),
                entry: Some(entry),
                ..ManualEntryResult::plain("created")
            })
        }
        Err(e) => {
            log::warn!("create_manual_entry: Odoo refused, nothing written: {e}");
            Ok(ManualEntryResult::rejected(&e))
        }
    }
}

/// Look up the task an Odoo line was last known to belong to, so a task change
/// can trigger the effective_hours recompute on both sides.
fn cached_task_for_line(conn: &rusqlite::Connection, odoo_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT task_id FROM cached_timesheet_entries WHERE odoo_id = ?1",
        rusqlite::params![odoo_id],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .filter(|id| *id != 0)
}

/// Edit an existing Odoo timesheet line.
///
/// Edits are never queued: `pending_timesheets` has create semantics only, so
/// an offline edit would need an operation log this feature does not have.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn update_timesheet_entry(
    odoo_id: i64,
    task_id: i64,
    project_id: i64,
    task_name: String,
    project_name: String,
    description: String,
    duration_hours: f64,
    date: String,
    original_date: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<ManualEntryResult> {
    validate_manual_entry(task_id, project_id, duration_hours, &date, &description)?;
    let description = effective_description(&description, &task_name);

    log::info!("update_timesheet_entry: line {odoo_id} -> task {task_id}, {duration_hours:.2}h on {date}");

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };
    let Some(client) = odoo_client else {
        return Err(AppError::General(
            "Reconnect to Odoo to edit this entry".into(),
        ));
    };

    let previous_task_id = {
        let db = state.db.lock().unwrap();
        cached_task_for_line(&db, odoo_id)
    };

    let mut values = HashMap::new();
    values.insert("name".into(), XmlRpcValue::String(description.clone()));
    values.insert("task_id".into(), XmlRpcValue::Int(task_id));
    values.insert("project_id".into(), XmlRpcValue::Int(project_id));
    values.insert("unit_amount".into(), XmlRpcValue::Double(duration_hours));
    values.insert("date".into(), XmlRpcValue::String(date.clone()));

    match client.update_timesheet_line(odoo_id, values).await {
        Ok(_) => {
            // Recompute both sides when the entry moved between tasks.
            client.recompute_task(task_id).await;
            if let Some(prev) = previous_task_id.filter(|prev| *prev != task_id) {
                client.recompute_task(prev).await;
            }

            let entry = TimesheetEntry {
                id: Some(odoo_id),
                task_id: Some(task_id),
                task_name,
                project_id: Some(project_id),
                project_name,
                description,
                hours: duration_hours,
                date: date.clone(),
                source: "odoo".into(),
            };

            emit_ledger_changed(&app, &date, "updated", Some(odoo_id), None);
            // Moving an entry between days mutates two days.
            if original_date != date && !original_date.is_empty() {
                emit_ledger_changed(&app, &original_date, "updated", Some(odoo_id), None);
                spawn_entries_refresh(app.clone(), original_date);
            }
            spawn_entries_refresh(app, date);

            Ok(ManualEntryResult {
                odoo_id: Some(odoo_id),
                entry: Some(entry),
                ..ManualEntryResult::plain("updated")
            })
        }
        Err(e) => {
            log::warn!("update_timesheet_entry: Odoo refused line {odoo_id}: {e}");
            Ok(ManualEntryResult::rejected(&e))
        }
    }
}

/// Delete an Odoo timesheet line. The first destructive Odoo operation the app
/// performs — never auto-retried.
#[tauri::command]
pub async fn delete_timesheet_entry(
    odoo_id: i64,
    task_id: Option<i64>,
    date: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<DeleteResult> {
    log::info!("delete_timesheet_entry: line {odoo_id} on {date}");

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };
    let Some(client) = odoo_client else {
        return Err(AppError::General(
            "Reconnect to Odoo to delete this entry".into(),
        ));
    };

    match client.unlink("account.analytic.line", vec![odoo_id]).await {
        Ok(ok) => {
            if !ok {
                log::warn!("delete_timesheet_entry: unlink of {odoo_id} returned a falsy result");
            }
        }
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            // Already gone in Odoo: still clean up locally so the row stops showing.
            if msg.contains("does not exist") || msg.contains("record not found") {
                log::info!("delete_timesheet_entry: line {odoo_id} already gone in Odoo");
            } else {
                return Err(e);
            }
        }
    }

    {
        let db = state.db.lock().unwrap();
        // Drop the phantom local-log row that would otherwise keep inflating the
        // recurring/forgotten/ranking heuristics forever.
        let _ = delete_log_by_odoo_id(&db, odoo_id);
        // Local cache invalidation; spawn_entries_refresh rebuilds the date.
        let _ = db.execute(
            "DELETE FROM cached_timesheet_entries WHERE odoo_id = ?1",
            rusqlite::params![odoo_id],
        );
    }

    if let Some(tid) = task_id.filter(|id| *id > 0) {
        client.recompute_task(tid).await;
    }

    emit_ledger_changed(&app, &date, "deleted", Some(odoo_id), None);
    spawn_entries_refresh(app, date);

    Ok(DeleteResult { deleted: true })
}

/// Rewrite a queued entry and put it back at the front of the queue.
///
/// This is the only way out of a 'rejected' row (e.g. "Timesheets cannot be
/// created on a private task"): `resolve_sync_entry` can skip/force/discard but
/// cannot change the payload. Purely local, so it works offline — which is
/// exactly when it is needed.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn update_pending_entry(
    entry_id: i64,
    task_id: i64,
    project_id: i64,
    task_name: String,
    project_name: String,
    description: String,
    duration_hours: f64,
    date: String,
    allow_duplicate: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    validate_manual_entry(task_id, project_id, duration_hours, &date, &description)?;
    let description = effective_description(&description, &task_name);

    log::info!("update_pending_entry: rewriting queued entry {entry_id} -> task {task_id} on {date}");

    {
        let db = state.db.lock().unwrap();
        update_pending(
            &db,
            entry_id,
            task_id,
            project_id,
            &task_name,
            &project_name,
            &description,
            duration_hours,
            &date,
            allow_duplicate,
        )?;
    }

    emit_ledger_changed(&app, &date, "pending_updated", None, Some(entry_id));
    Ok(())
}

/// Queued (not-yet-synced) entries for a date.
///
/// Additive to `get_entries_for_date`, which is left untouched: pending hours
/// are shown as their own figure and never folded into the Odoo total.
#[tauri::command]
pub async fn get_pending_for_date(
    date: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<PendingTimesheet>> {
    let db = state.db.lock().unwrap();
    crate::db::timesheets::get_pending_for_date(&db, &date)
}
