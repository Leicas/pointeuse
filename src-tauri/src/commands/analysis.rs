use serde::Serialize;

use tauri::{Emitter, Manager};

use crate::db::cache;
use crate::db::timesheets::{get_forgotten_tasks, get_recurring_tasks_missing_today, TaskPattern};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Max cache age before triggering background refresh for analysis data.
const ANALYSIS_CACHE_MAX_AGE_TODAY: i64 = 60;
const ANALYSIS_CACHE_MAX_AGE_PAST: i64 = 3600;

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceBlock {
    pub check_in: String,
    pub check_out: Option<String>,
    pub worked_hours: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimesheetBlock {
    pub task_id: Option<i64>,
    pub task_name: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub description: String,
    pub hours: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub message: String,
    pub detail: Option<String>,
    pub suggestion_type: String, // "add_time" | "split_gap" | "missing_recurring" | "info" | "all_good"
    pub task_id: Option<i64>,
    pub task_name: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub hours: Option<f64>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayAnalysis {
    pub date: String,
    pub attendance_blocks: Vec<AttendanceBlock>,
    pub timesheet_blocks: Vec<TimesheetBlock>,
    pub total_attendance_hours: f64,
    pub total_timesheet_hours: f64,
    pub gap_hours: f64,
    pub suggestions: Vec<Suggestion>,
}

fn max_age_for_date(date: &str) -> i64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    if date == today { ANALYSIS_CACHE_MAX_AGE_TODAY } else { ANALYSIS_CACHE_MAX_AGE_PAST }
}

/// Build a DayAnalysis from attendance blocks and timesheet entries + local patterns.
fn build_analysis(
    date: &str,
    attendance_blocks: Vec<AttendanceBlock>,
    timesheet_blocks: Vec<TimesheetBlock>,
    recurring_missing: &[TaskPattern],
    forgotten: &[TaskPattern],
) -> DayAnalysis {
    let total_attendance_hours: f64 = attendance_blocks.iter().map(|a| a.worked_hours).sum();
    let total_timesheet_hours: f64 = timesheet_blocks.iter().map(|t| t.hours).sum();
    let gap_hours = total_attendance_hours - total_timesheet_hours;

    let suggestions = generate_suggestions(
        gap_hours,
        total_attendance_hours,
        total_timesheet_hours,
        &timesheet_blocks,
        date,
        recurring_missing,
        forgotten,
    );

    DayAnalysis {
        date: date.to_string(),
        attendance_blocks,
        timesheet_blocks,
        total_attendance_hours,
        total_timesheet_hours,
        gap_hours,
        suggestions,
    }
}

/// Spawn a background refresh for analysis data (attendance + timesheets).
fn spawn_analysis_refresh(app: tauri::AppHandle, date: String) {
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("cache_sync_start", format!("analysis:{date}"));
        let state: tauri::State<'_, AppState> = app.state::<AppState>();
        let client: Option<crate::odoo::client::OdooClient> = {
            let odoo = state.odoo.lock().unwrap();
            odoo.clone()
        };
        let Some(client) = client else {
            let _ = app.emit("cache_sync_done", format!("analysis:{date}"));
            return;
        };

        let attendance_result = client.get_today_attendance(&date).await;
        let timesheet_result = client.get_today_timesheets(&date).await;

        let Ok(attendance_raw) = attendance_result else { return };
        let Ok(timesheets_raw) = timesheet_result else { return };

        let attendance_blocks: Vec<AttendanceBlock> = attendance_raw
            .into_iter()
            .map(|(check_in, check_out, worked_hours)| AttendanceBlock {
                check_in, check_out, worked_hours,
            })
            .collect();

        let ts_entries = crate::commands::timesheet::convert_odoo_entries(timesheets_raw);
        let timesheet_blocks: Vec<TimesheetBlock> = ts_entries
            .iter()
            .map(|e| TimesheetBlock {
                task_id: e.task_id,
                task_name: e.task_name.clone(),
                project_id: e.project_id,
                project_name: e.project_name.clone(),
                description: e.description.clone(),
                hours: e.hours,
            })
            .collect();

        // Save to cache
        {
            let db = state.db.lock().unwrap();
            let _ = cache::cache_attendance(&db, &date, &attendance_blocks);
            let _ = cache::cache_timesheet_entries(&db, &date, &ts_entries);
        }

        // Build analysis with local patterns
        let (recurring_missing, forgotten) = {
            let db = state.db.lock().unwrap();
            let recurring = get_recurring_tasks_missing_today(&db, &date, 10, 3).unwrap_or_default();
            let forgotten = get_forgotten_tasks(&db, &date, 3).unwrap_or_default();
            (recurring, forgotten)
        };

        let analysis = build_analysis(&date, attendance_blocks, timesheet_blocks, &recurring_missing, &forgotten);
        log::info!("cache: refreshed analysis for {date}");
        let _ = app.emit("analysis_refreshed", &analysis);
        let _ = app.emit("cache_sync_done", format!("analysis:{date}"));
    });
}

#[tauri::command]
pub async fn get_day_analysis(
    date: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<DayAnalysis> {
    // Check if we have cached attendance + timesheets for this date
    let (cached_attendance, cached_timesheets, att_age, ts_age) = {
        let db = state.db.lock().unwrap();
        let att = cache::get_cached_attendance(&db, &date).unwrap_or_default();
        let ts = cache::get_cached_timesheet_entries(&db, &date).unwrap_or_default();
        let att_age = cache::get_cache_age_secs(&db, &format!("att:{date}")).unwrap_or(None);
        let ts_age = cache::get_cache_age_secs(&db, &format!("ts:{date}")).unwrap_or(None);
        (att, ts, att_age, ts_age)
    };

    let has_cache = !cached_attendance.is_empty() || !cached_timesheets.is_empty();
    let max_age = max_age_for_date(&date);
    let is_stale = att_age.map_or(true, |a| a > max_age) || ts_age.map_or(true, |a| a > max_age);

    if has_cache {
        // Convert cached timesheets to TimesheetBlocks
        let timesheet_blocks: Vec<TimesheetBlock> = cached_timesheets
            .iter()
            .map(|e| TimesheetBlock {
                task_id: e.task_id,
                task_name: e.task_name.clone(),
                project_id: e.project_id,
                project_name: e.project_name.clone(),
                description: e.description.clone(),
                hours: e.hours,
            })
            .collect();

        let (recurring_missing, forgotten) = {
            let db = state.db.lock().unwrap();
            let recurring = get_recurring_tasks_missing_today(&db, &date, 10, 3).unwrap_or_default();
            let forgotten = get_forgotten_tasks(&db, &date, 3).unwrap_or_default();
            (recurring, forgotten)
        };

        let analysis = build_analysis(&date, cached_attendance, timesheet_blocks, &recurring_missing, &forgotten);

        // Refresh in background if stale
        if is_stale {
            spawn_analysis_refresh(app, date);
        }
        return Ok(analysis);
    }

    // No cache — fetch from Odoo synchronously
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    // Fetch attendance and timesheets in parallel
    let (attendance_result, timesheet_result) = tokio::join!(
        client.get_today_attendance(&date),
        client.get_today_timesheets(&date),
    );

    let attendance_raw = attendance_result?;
    let timesheets_raw = timesheet_result?;

    let attendance_blocks: Vec<AttendanceBlock> = attendance_raw
        .into_iter()
        .map(|(check_in, check_out, worked_hours)| AttendanceBlock {
            check_in,
            check_out,
            worked_hours,
        })
        .collect();

    let ts_entries = crate::commands::timesheet::convert_odoo_entries(timesheets_raw);
    let timesheet_blocks: Vec<TimesheetBlock> = ts_entries
        .iter()
        .map(|e| TimesheetBlock {
            task_id: e.task_id,
            task_name: e.task_name.clone(),
            project_id: e.project_id,
            project_name: e.project_name.clone(),
            description: e.description.clone(),
            hours: e.hours,
        })
        .collect();

    // Save to cache
    {
        let db = state.db.lock().unwrap();
        let _ = cache::cache_attendance(&db, &date, &attendance_blocks);
        let _ = cache::cache_timesheet_entries(&db, &date, &ts_entries);
    }

    // Fetch historical patterns from local DB
    let (recurring_missing, forgotten) = {
        let db = state.db.lock().unwrap();
        let recurring = get_recurring_tasks_missing_today(&db, &date, 10, 3).unwrap_or_default();
        let forgotten = get_forgotten_tasks(&db, &date, 3).unwrap_or_default();
        (recurring, forgotten)
    };

    Ok(build_analysis(&date, attendance_blocks, timesheet_blocks, &recurring_missing, &forgotten))
}

#[allow(clippy::too_many_arguments)]
fn generate_suggestions(
    gap_hours: f64,
    total_attendance: f64,
    total_timesheet: f64,
    entries: &[TimesheetBlock],
    _date: &str,
    recurring_missing: &[TaskPattern],
    forgotten: &[TaskPattern],
) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();
    let mut id_counter = 0u32;
    let mut next_id = || {
        id_counter += 1;
        format!("s{id_counter}")
    };

    // ── Case: time is balanced ──────────────────────────────────────
    if gap_hours.abs() <= 0.25 && total_attendance > 0.0 && total_timesheet > 0.0 {
        // Even if balanced, check for missing recurring tasks
        if let Some(best) = recurring_missing.first() {
            suggestions.push(Suggestion {
                id: next_id(),
                message: format!(
                    "\"{}\" is a daily task you haven't logged yet",
                    best.task_name
                ),
                detail: Some(format!(
                    "Logged {} of last 10 workdays, avg {:.1}h",
                    best.days_logged, best.avg_hours
                )),
                suggestion_type: "missing_recurring".into(),
                task_id: Some(best.task_id),
                task_name: Some(best.task_name.clone()),
                project_id: None,
                project_name: Some(best.project_name.clone()),
                hours: Some(round_quarter(best.avg_hours)),
                description: Some(best.task_name.clone()),
            });
        } else {
            suggestions.push(Suggestion {
                id: next_id(),
                message: "Time is balanced".into(),
                detail: Some(format!(
                    "Presence ({:.1}h) matches logged time ({:.1}h)",
                    total_attendance, total_timesheet
                )),
                suggestion_type: "all_good".into(),
                task_id: None,
                task_name: None,
                project_id: None,
                project_name: None,
                hours: None,
                description: None,
            });
        }
        suggestions.truncate(3);
        return suggestions;
    }

    // ── Case: gap > 0.25h (presence exceeds logged) ─────────────────
    if gap_hours > 0.25 {
        // Priority 1: Suggest a missing recurring task to fill the gap
        if let Some(best) = recurring_missing.first() {
            let suggested_hours = round_quarter(best.avg_hours.min(gap_hours));
            suggestions.push(Suggestion {
                id: next_id(),
                message: format!(
                    "Log {:.1}h to \"{}\" (daily task)",
                    suggested_hours, best.task_name
                ),
                detail: Some(format!(
                    "You log this {} of 10 workdays, avg {:.1}h/day",
                    best.days_logged, best.avg_hours
                )),
                suggestion_type: "add_time".into(),
                task_id: Some(best.task_id),
                task_name: Some(best.task_name.clone()),
                project_id: None,
                project_name: Some(best.project_name.clone()),
                hours: Some(suggested_hours),
                description: Some(best.task_name.clone()),
            });
        }

        // Priority 2: Suggest a forgotten task (worked recently but not today)
        let remaining_gap = if !recurring_missing.is_empty() {
            gap_hours - recurring_missing[0].avg_hours.min(gap_hours)
        } else {
            gap_hours
        };

        if remaining_gap > 0.25 {
            // Find forgotten task not already suggested
            let already_suggested: Vec<i64> = recurring_missing.iter().map(|t| t.task_id).collect();
            if let Some(forgot) = forgotten
                .iter()
                .find(|t| !already_suggested.contains(&t.task_id))
            {
                let suggested_hours = round_quarter(forgot.avg_hours.min(remaining_gap));
                suggestions.push(Suggestion {
                    id: next_id(),
                    message: format!(
                        "Add {:.1}h to \"{}\" (worked on recently)",
                        suggested_hours, forgot.task_name
                    ),
                    detail: Some(format!(
                        "Last logged {}, avg {:.1}h/day",
                        forgot.last_date, forgot.avg_hours
                    )),
                    suggestion_type: "add_time".into(),
                    task_id: Some(forgot.task_id),
                    task_name: Some(forgot.task_name.clone()),
                    project_id: None,
                    project_name: Some(forgot.project_name.clone()),
                    hours: Some(suggested_hours),
                    description: Some(forgot.task_name.clone()),
                });
            }
        }

        // Priority 3: Fall back to splitting across existing entries
        if suggestions.is_empty() && !entries.is_empty() {
            let largest = entries
                .iter()
                .filter(|e| e.task_id.is_some())
                .max_by(|a, b| {
                    a.hours
                        .partial_cmp(&b.hours)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some(largest) = largest {
                suggestions.push(Suggestion {
                    id: next_id(),
                    message: format!("Add {:.1}h to \"{}\"", round_quarter(gap_hours), largest.task_name),
                    detail: Some("Covers the gap between presence and logged time".into()),
                    suggestion_type: "add_time".into(),
                    task_id: largest.task_id,
                    task_name: Some(largest.task_name.clone()),
                    project_id: largest.project_id,
                    project_name: Some(largest.project_name.clone()),
                    hours: Some(round_quarter(gap_hours)),
                    description: Some(largest.task_name.clone()),
                });
            }
        }

        // If still nothing and no entries at all
        if suggestions.is_empty() && entries.is_empty() {
            suggestions.push(Suggestion {
                id: next_id(),
                message: format!(
                    "You were present {:.1}h but logged nothing",
                    total_attendance
                ),
                detail: Some("Select a task and log your time".into()),
                suggestion_type: "info".into(),
                task_id: None,
                task_name: None,
                project_id: None,
                project_name: None,
                hours: None,
                description: None,
            });
        }

        // Add any additional recurring tasks as lower-priority suggestions
        for rt in recurring_missing.iter().skip(1).take(1) {
            if suggestions.len() >= 3 {
                break;
            }
            suggestions.push(Suggestion {
                id: next_id(),
                message: format!(
                    "\"{}\" is also a regular task not logged today",
                    rt.task_name
                ),
                detail: Some(format!(
                    "Logged {} of 10 workdays, avg {:.1}h",
                    rt.days_logged, rt.avg_hours
                )),
                suggestion_type: "missing_recurring".into(),
                task_id: Some(rt.task_id),
                task_name: Some(rt.task_name.clone()),
                project_id: None,
                project_name: Some(rt.project_name.clone()),
                hours: Some(round_quarter(rt.avg_hours)),
                description: Some(rt.task_name.clone()),
            });
        }
    }

    // ── Case: gap < -0.25h (logged exceeds presence) ────────────────
    if gap_hours < -0.25 {
        suggestions.push(Suggestion {
            id: next_id(),
            message: format!(
                "Logged {:.1}h more than attendance ({:.1}h)",
                -gap_hours, total_attendance
            ),
            detail: Some("Check if your attendance was recorded correctly".into()),
            suggestion_type: "info".into(),
            task_id: None,
            task_name: None,
            project_id: None,
            project_name: None,
            hours: None,
            description: None,
        });
    }

    suggestions.truncate(3);
    suggestions
}

/// Round to nearest quarter hour.
fn round_quarter(h: f64) -> f64 {
    (h * 4.0).round() / 4.0
}
