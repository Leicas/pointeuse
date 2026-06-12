use chrono::{self, NaiveDate};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

// ── Data types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTimesheet {
    pub id: i64,
    pub task_id: i64,
    pub project_id: i64,
    pub description: String,
    pub duration_hours: f64,
    pub date: String,
    pub status: String,
    pub odoo_id: Option<i64>,
    pub retry_count: i64,
    pub last_error: Option<String>,
    pub last_attempt_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimesheetLogEntry {
    pub id: Option<i64>,
    pub task_id: i64,
    pub task_name: String,
    pub project_name: String,
    pub description: String,
    pub hours: f64,
    pub date: String,
    pub synced_at: Option<String>,
    pub odoo_id: Option<i64>,
}

// ── Pending queue ────────────────────────────────────────────────────

/// Insert a new pending timesheet and return its row id.
pub fn queue_timesheet(
    conn: &Connection,
    task_id: i64,
    project_id: i64,
    description: &str,
    duration_hours: f64,
    date: &str,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO pending_timesheets (task_id, project_id, description, duration_hours, date)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task_id, project_id, description, duration_hours, date],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Return all timesheets that have not yet been synced.
pub fn get_pending_timesheets(conn: &Connection) -> AppResult<Vec<PendingTimesheet>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, project_id, description, duration_hours, date,
                status, odoo_id, retry_count, last_error, last_attempt_at, created_at
         FROM pending_timesheets
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PendingTimesheet {
            id: row.get(0)?,
            task_id: row.get(1)?,
            project_id: row.get(2)?,
            description: row.get(3)?,
            duration_hours: row.get(4)?,
            date: row.get(5)?,
            status: row.get(6)?,
            odoo_id: row.get(7)?,
            retry_count: row.get(8)?,
            last_error: row.get(9)?,
            last_attempt_at: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Remove a pending entry after successful sync (legacy, kept for potential external use).
#[allow(dead_code)]
pub fn mark_synced(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM pending_timesheets WHERE id = ?1", params![id])?;
    Ok(())
}

/// Return the number of pending (un-synced) timesheets.
#[allow(dead_code)]
pub fn get_pending_count(conn: &Connection) -> AppResult<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_timesheets WHERE status NOT IN ('synced')",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

// ── Enhanced sync operations ────────────────────────────────────────

/// Maximum number of automatic retries before an entry requires manual action.
const MAX_AUTO_RETRIES: i64 = 10;

/// Compute backoff delay in seconds for a given retry count.
/// 0 retries = 0s, 1 = 30s, 2 = 60s, 3 = 120s, ... capped at 3600s.
fn backoff_secs(retry_count: i64) -> f64 {
    if retry_count <= 0 {
        return 0.0;
    }
    let base = 30.0_f64 * 2.0_f64.powi((retry_count - 1) as i32);
    base.min(3600.0)
}

/// Claim entries eligible for sync by setting their status to 'syncing'.
/// Returns the claimed entries. Respects retry backoff.
/// Also recovers entries stuck in 'syncing' for over 5 minutes (crashed sync).
pub fn claim_entries_for_sync(conn: &Connection) -> AppResult<Vec<PendingTimesheet>> {
    // Recover entries stuck in 'syncing' for over 5 minutes (crashed sync).
    conn.execute(
        "UPDATE pending_timesheets SET status = 'pending'
         WHERE status = 'syncing'
           AND last_attempt_at IS NOT NULL
           AND julianday('now') - julianday(last_attempt_at) > 300.0 / 86400.0",
        [],
    )?;

    // Fetch candidates (pending/failed with retries remaining).
    let mut stmt = conn.prepare(
        "SELECT id, task_id, project_id, description, duration_hours, date,
                status, odoo_id, retry_count, last_error, last_attempt_at, created_at
         FROM pending_timesheets
         WHERE status IN ('pending', 'failed')
           AND retry_count < ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![MAX_AUTO_RETRIES], |row| {
        Ok(PendingTimesheet {
            id: row.get(0)?,
            task_id: row.get(1)?,
            project_id: row.get(2)?,
            description: row.get(3)?,
            duration_hours: row.get(4)?,
            date: row.get(5)?,
            status: row.get(6)?,
            odoo_id: row.get(7)?,
            retry_count: row.get(8)?,
            last_error: row.get(9)?,
            last_attempt_at: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;

    let now = chrono::Utc::now();
    let mut eligible_ids = Vec::new();
    let mut eligible_entries = Vec::new();

    for row in rows {
        let entry = row?;
        // Check backoff
        let backoff = backoff_secs(entry.retry_count);
        let elapsed = entry
            .last_attempt_at
            .as_deref()
            .and_then(|ts| chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok())
            .map(|dt| (now - dt.and_utc()).num_seconds() as f64)
            .unwrap_or(f64::MAX);

        if elapsed >= backoff {
            eligible_ids.push(entry.id);
            eligible_entries.push(entry);
        }
    }

    // Claim all eligible entries atomically.
    if !eligible_ids.is_empty() {
        let placeholders: String = eligible_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE pending_timesheets SET status = 'syncing', last_attempt_at = datetime('now')
             WHERE id IN ({placeholders})"
        );
        let mut update_stmt = conn.prepare(&sql)?;
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = eligible_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        update_stmt.execute(param_refs.as_slice())?;

        // Update the status in our returned entries
        for entry in &mut eligible_entries {
            entry.status = "syncing".to_string();
        }
    }

    Ok(eligible_entries)
}

/// Mark an entry as successfully synced to Odoo.
pub fn mark_entry_synced(conn: &Connection, id: i64, odoo_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE pending_timesheets
         SET status = 'synced', odoo_id = ?2, last_error = NULL, last_attempt_at = datetime('now')
         WHERE id = ?1",
        params![id, odoo_id],
    )?;
    Ok(())
}

/// Mark an entry as failed with error details.
pub fn mark_entry_failed(conn: &Connection, id: i64, error: &str, is_permanent: bool) -> AppResult<()> {
    let status = if is_permanent { "rejected" } else { "failed" };
    conn.execute(
        "UPDATE pending_timesheets
         SET status = ?2, retry_count = retry_count + 1, last_error = ?3, last_attempt_at = datetime('now')
         WHERE id = ?1",
        params![id, status, error],
    )?;
    Ok(())
}

/// Mark an entry as a suspected duplicate, storing the matching Odoo entry ID.
pub fn mark_entry_duplicate(conn: &Connection, id: i64, matching_odoo_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE pending_timesheets
         SET status = 'duplicate', odoo_id = ?2, last_attempt_at = datetime('now'),
             last_error = 'Matching entry found in Odoo (id=' || ?2 || ')'
         WHERE id = ?1",
        params![id, matching_odoo_id],
    )?;
    Ok(())
}

/// Release a claimed entry back to pending (e.g., if sync was aborted).
pub fn release_entry(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE pending_timesheets SET status = 'pending' WHERE id = ?1 AND status = 'syncing'",
        params![id],
    )?;
    Ok(())
}

/// Resolve a duplicate or rejected entry by user action.
/// action: "skip" = mark as synced (with the existing odoo_id), "force" = reset to pending for re-sync, "discard" = delete.
pub fn resolve_entry(conn: &Connection, id: i64, action: &str) -> AppResult<()> {
    match action {
        "skip" => {
            conn.execute(
                "UPDATE pending_timesheets SET status = 'synced', last_attempt_at = datetime('now') WHERE id = ?1",
                params![id],
            )?;
        }
        "force" => {
            conn.execute(
                "UPDATE pending_timesheets SET status = 'pending', retry_count = 0, odoo_id = NULL, last_error = NULL WHERE id = ?1",
                params![id],
            )?;
        }
        "discard" => {
            conn.execute("DELETE FROM pending_timesheets WHERE id = ?1", params![id])?;
        }
        _ => {
            return Err(crate::error::AppError::General(format!("Unknown resolve action: {action}")));
        }
    }
    Ok(())
}

/// Get entries that need user attention (duplicates, rejected, exhausted retries).
pub fn get_entries_needing_review(conn: &Connection) -> AppResult<Vec<PendingTimesheet>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, project_id, description, duration_hours, date,
                status, odoo_id, retry_count, last_error, last_attempt_at, created_at
         FROM pending_timesheets
         WHERE status IN ('duplicate', 'rejected')
            OR (status = 'failed' AND retry_count >= ?1)
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![MAX_AUTO_RETRIES], |row| {
        Ok(PendingTimesheet {
            id: row.get(0)?,
            task_id: row.get(1)?,
            project_id: row.get(2)?,
            description: row.get(3)?,
            duration_hours: row.get(4)?,
            date: row.get(5)?,
            status: row.get(6)?,
            odoo_id: row.get(7)?,
            retry_count: row.get(8)?,
            last_error: row.get(9)?,
            last_attempt_at: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Get counts by status for the sync status display.
pub fn get_sync_status_counts(conn: &Connection) -> AppResult<SyncStatusCounts> {
    let mut counts = SyncStatusCounts::default();
    let mut stmt = conn.prepare(
        "SELECT status, COUNT(*) FROM pending_timesheets GROUP BY status",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, count) = row?;
        match status.as_str() {
            "pending" => counts.pending = count,
            "syncing" => counts.syncing = count,
            "synced" => counts.synced = count,
            "failed" => counts.failed = count,
            "duplicate" => counts.duplicate = count,
            "rejected" => counts.rejected = count,
            _ => {}
        }
    }
    Ok(counts)
}

/// Counts by status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStatusCounts {
    pub pending: i64,
    pub syncing: i64,
    pub synced: i64,
    pub failed: i64,
    pub duplicate: i64,
    pub rejected: i64,
}

/// Clean up old synced entries (keep for audit trail, remove after 7 days).
pub fn cleanup_old_synced(conn: &Connection) -> AppResult<usize> {
    let deleted = conn.execute(
        "DELETE FROM pending_timesheets
         WHERE status = 'synced'
           AND last_attempt_at < datetime('now', '-7 days')",
        [],
    )?;
    Ok(deleted)
}

// ── Timesheet log ────────────────────────────────────────────────────

/// Append an entry to the local sync log.
#[allow(clippy::too_many_arguments)]
pub fn add_to_log(
    conn: &Connection,
    task_id: i64,
    task_name: &str,
    project_name: &str,
    description: &str,
    hours: f64,
    date: &str,
    odoo_id: Option<i64>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO timesheet_log (task_id, task_name, project_name, description, hours, date, odoo_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![task_id, task_name, project_name, description, hours, date, odoo_id],
    )?;
    Ok(())
}

/// Return all log entries whose `date` matches today (UTC).
pub fn get_today_log(conn: &Connection) -> AppResult<Vec<TimesheetLogEntry>> {
    get_log_for_date(conn, None)
}

/// Return all log entries for a specific date. If None, uses today.
pub fn get_log_for_date(conn: &Connection, date: Option<&str>) -> AppResult<Vec<TimesheetLogEntry>> {
    // Resolve date: use provided or today
    let resolved_date = match date {
        Some(d) => d.to_string(),
        None => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    let mut stmt = conn.prepare(
        "SELECT id, task_id, task_name, project_name, description, hours, date, synced_at, odoo_id
         FROM timesheet_log
         WHERE date = ?1
         ORDER BY synced_at DESC",
    )?;
    let rows = stmt.query_map(params![resolved_date], |row| {
        Ok(TimesheetLogEntry {
            id: Some(row.get(0)?),
            task_id: row.get(1)?,
            task_name: row.get(2)?,
            project_name: row.get(3)?,
            description: row.get(4)?,
            hours: row.get(5)?,
            date: row.get(6)?,
            synced_at: row.get(7)?,
            odoo_id: row.get(8)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ── Smart heuristic queries ──────────────────────────────────────────

/// Data about a task's historical pattern.
#[derive(Debug, Clone, Serialize)]
pub struct TaskPattern {
    pub task_id: i64,
    pub task_name: String,
    pub project_name: String,
    pub days_logged: i64,     // distinct days in window
    pub avg_hours: f64,       // average hours per logged day
    pub last_date: String,    // most recent date logged
}

/// Tasks logged on at least `min_days` of the last `window_days` weekdays but NOT today.
/// These are "recurring tasks the user probably forgot".
pub fn get_recurring_tasks_missing_today(
    conn: &Connection,
    today: &str,
    window_days: u32,
    min_days: u32,
) -> AppResult<Vec<TaskPattern>> {
    let mut stmt = conn.prepare(
        "SELECT
            task_id,
            task_name,
            project_name,
            COUNT(DISTINCT date) AS days_logged,
            ROUND(SUM(hours) / COUNT(DISTINCT date), 2) AS avg_hours,
            MAX(date) AS last_date
         FROM timesheet_log
         WHERE date >= date(?1, '-' || ?2 || ' days')
           AND date < ?1
           AND CAST(strftime('%w', date) AS INTEGER) NOT IN (0, 6)
         GROUP BY task_id
         HAVING days_logged >= ?3
           AND task_id NOT IN (
               SELECT DISTINCT task_id FROM timesheet_log WHERE date = ?1
           )
         ORDER BY days_logged DESC, avg_hours DESC
         LIMIT 5",
    )?;
    let rows = stmt.query_map(
        params![today, window_days, min_days],
        |row| {
            Ok(TaskPattern {
                task_id: row.get(0)?,
                task_name: row.get(1)?,
                project_name: row.get(2)?,
                days_logged: row.get(3)?,
                avg_hours: row.get(4)?,
                last_date: row.get(5)?,
            })
        },
    )?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Tasks worked on in the last N days but not today (forgotten tasks).
pub fn get_forgotten_tasks(
    conn: &Connection,
    today: &str,
    lookback_days: u32,
) -> AppResult<Vec<TaskPattern>> {
    let mut stmt = conn.prepare(
        "SELECT
            task_id,
            task_name,
            project_name,
            COUNT(DISTINCT date) AS days_logged,
            ROUND(SUM(hours) / COUNT(DISTINCT date), 2) AS avg_hours,
            MAX(date) AS last_date
         FROM timesheet_log
         WHERE date >= date(?1, '-' || ?2 || ' days')
           AND date < ?1
         GROUP BY task_id
         HAVING task_id NOT IN (
             SELECT DISTINCT task_id FROM timesheet_log WHERE date = ?1
         )
         ORDER BY last_date DESC, days_logged DESC
         LIMIT 10",
    )?;
    let rows = stmt.query_map(params![today, lookback_days], |row| {
        Ok(TaskPattern {
            task_id: row.get(0)?,
            task_name: row.get(1)?,
            project_name: row.get(2)?,
            days_logged: row.get(3)?,
            avg_hours: row.get(4)?,
            last_date: row.get(5)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Ranking data for smart task suggestions.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TaskRankingData {
    pub task_id: i64,
    pub task_name: String,
    pub project_name: String,
    pub days_since_last: i64,
    pub frequency_30d: i64,
    pub weekday_frequency: i64,
    pub avg_hours: f64,
    pub total_days: i64,
}

/// Get ranking data for all tasks with recent history.
pub fn get_task_ranking(
    conn: &Connection,
    today: &str,
    current_weekday: u32, // 0=Sun, 1=Mon, ..., 6=Sat (strftime %w)
) -> AppResult<Vec<TaskRankingData>> {
    let mut stmt = conn.prepare(
        "SELECT
            task_id,
            task_name,
            project_name,
            CAST(julianday(?1) - julianday(MAX(date)) AS INTEGER) AS days_since_last,
            COUNT(DISTINCT CASE WHEN date >= date(?1, '-30 days') THEN date END) AS freq_30d,
            COUNT(DISTINCT CASE
                WHEN CAST(strftime('%w', date) AS INTEGER) = ?2
                 AND date >= date(?1, '-60 days')
                THEN date END) AS weekday_freq,
            ROUND(SUM(hours) / MAX(COUNT(DISTINCT date), 1), 2) AS avg_hours,
            COUNT(DISTINCT date) AS total_days
         FROM timesheet_log
         WHERE date >= date(?1, '-90 days')
         GROUP BY task_id
         HAVING days_since_last <= 60
         ORDER BY days_since_last ASC, freq_30d DESC
         LIMIT 30",
    )?;
    let rows = stmt.query_map(params![today, current_weekday], |row| {
        Ok(TaskRankingData {
            task_id: row.get(0)?,
            task_name: row.get(1)?,
            project_name: row.get(2)?,
            days_since_last: row.get(3)?,
            frequency_30d: row.get(4)?,
            weekday_frequency: row.get(5)?,
            avg_hours: row.get(6)?,
            total_days: row.get(7)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Return daily totals for a month from local log.
pub fn get_monthly_log_summary(
    conn: &Connection,
    year: i32,
    month: u32,
) -> AppResult<Vec<(String, f64, u32)>> {
    let start = format!("{year:04}-{month:02}-01");
    let end = {
        let next = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        };
        next.unwrap().pred_opt().unwrap().format("%Y-%m-%d").to_string()
    };
    let mut stmt = conn.prepare(
        "SELECT date, SUM(hours), COUNT(*)
         FROM timesheet_log
         WHERE date >= ?1 AND date <= ?2
         GROUP BY date
         ORDER BY date",
    )?;
    let rows = stmt.query_map(params![start, end], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, u32>(2)?,
        ))
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
