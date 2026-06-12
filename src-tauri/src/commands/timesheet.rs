use chrono::NaiveDate;
use serde::Serialize;

use tauri::{Emitter, Manager};

use crate::db::cache;
use crate::db::timesheets::{add_to_log, get_log_for_date, get_monthly_log_summary, get_today_log, queue_timesheet};
use crate::error::AppResult;
use crate::odoo::models::OdooTimesheetEntry;
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
) {
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
                        }
                        Err(e2) => {
                            log::error!("log_time_with_fallback: default task also failed, queuing: {e2}");
                            let state = app.state::<AppState>();
                            let db = state.db.lock().unwrap();
                            let _ = crate::db::timesheets::queue_timesheet(&db, dt.task_id, dt.project_id, &desc, hours, date);
                            let _ = crate::db::timesheets::add_to_log(&db, dt.task_id, &dt.task_name, &dt.project_name, &desc, hours, date, None);
                        }
                    }
                } else {
                    log::error!("log_time_with_fallback: private task rejected, no default task configured. Queuing: {e}");
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = crate::db::timesheets::queue_timesheet(&db, task_id, project_id, task_name, hours, date);
                    let _ = crate::db::timesheets::add_to_log(&db, task_id, task_name, project_name, task_name, hours, date, None);
                }
            } else {
                log::error!("log_time_with_fallback: Odoo failed, queuing: {e}");
                let state = app.state::<AppState>();
                let db = state.db.lock().unwrap();
                let _ = crate::db::timesheets::queue_timesheet(&db, task_id, project_id, task_name, hours, date);
                let _ = crate::db::timesheets::add_to_log(&db, task_id, task_name, project_name, task_name, hours, date, None);
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
                                queue_timesheet(&db, dt.task_id, dt.project_id, &desc, duration_hours, &date)?;
                                (None, true)
                            }
                        }
                    } else {
                        log::error!("log_time: private task rejected, no default task configured. Queuing (will be discarded on sync): {e}");
                        let db = state.db.lock().unwrap();
                        queue_timesheet(&db, task_id, project_id, &description, duration_hours, &date)?;
                        (None, true)
                    }
                } else {
                    log::error!("log_time: Odoo failed, queuing: {e}");
                    let db = state.db.lock().unwrap();
                    queue_timesheet(&db, task_id, project_id, &description, duration_hours, &date)?;
                    (None, true)
                }
            }
        }
    } else {
        log::info!("log_time: no Odoo connection, queuing");
        let db = state.db.lock().unwrap();
        queue_timesheet(&db, task_id, project_id, &description, duration_hours, &date)?;
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
