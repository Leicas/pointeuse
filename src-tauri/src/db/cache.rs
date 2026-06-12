use rusqlite::{params, Connection};

use crate::commands::analysis::AttendanceBlock;
use crate::commands::timesheet::TimesheetEntry;
use crate::error::AppResult;

// ── Timesheet entry cache ───────────────────────────────────────────

/// Replace all cached timesheet entries for a given date with fresh Odoo data.
pub fn cache_timesheet_entries(conn: &Connection, date: &str, entries: &[TimesheetEntry]) -> AppResult<()> {
    conn.execute("DELETE FROM cached_timesheet_entries WHERE date = ?1", params![date])?;
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO cached_timesheet_entries
         (odoo_id, task_id, task_name, project_id, project_name, description, hours, date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for e in entries {
        stmt.execute(params![
            e.id.unwrap_or(0),
            e.task_id.unwrap_or(0),
            e.task_name,
            e.project_id.unwrap_or(0),
            e.project_name,
            e.description,
            e.hours,
            e.date,
        ])?;
    }
    set_cache_updated(conn, &format!("ts:{date}"))?;
    Ok(())
}

/// Read cached timesheet entries for a single date.
pub fn get_cached_timesheet_entries(conn: &Connection, date: &str) -> AppResult<Vec<TimesheetEntry>> {
    let mut stmt = conn.prepare(
        "SELECT odoo_id, task_id, task_name, project_id, project_name, description, hours, date
         FROM cached_timesheet_entries
         WHERE date = ?1
         ORDER BY hours DESC",
    )?;
    let rows = stmt.query_map(params![date], |row| {
        let odoo_id: i64 = row.get(0)?;
        let task_id: i64 = row.get(1)?;
        let project_id: i64 = row.get(3)?;
        Ok(TimesheetEntry {
            id: if odoo_id != 0 { Some(odoo_id) } else { None },
            task_id: if task_id != 0 { Some(task_id) } else { None },
            task_name: row.get(2)?,
            project_id: if project_id != 0 { Some(project_id) } else { None },
            project_name: row.get(4)?,
            description: row.get(5)?,
            hours: row.get(6)?,
            date: row.get(7)?,
            source: "cache".into(),
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Read cached timesheet entries for a date range (for monthly summary).
pub fn get_cached_timesheet_range(
    conn: &Connection,
    start: &str,
    end: &str,
) -> AppResult<Vec<TimesheetEntry>> {
    let mut stmt = conn.prepare(
        "SELECT odoo_id, task_id, task_name, project_id, project_name, description, hours, date
         FROM cached_timesheet_entries
         WHERE date >= ?1 AND date <= ?2
         ORDER BY date, hours DESC",
    )?;
    let rows = stmt.query_map(params![start, end], |row| {
        let odoo_id: i64 = row.get(0)?;
        let task_id: i64 = row.get(1)?;
        let project_id: i64 = row.get(3)?;
        Ok(TimesheetEntry {
            id: if odoo_id != 0 { Some(odoo_id) } else { None },
            task_id: if task_id != 0 { Some(task_id) } else { None },
            task_name: row.get(2)?,
            project_id: if project_id != 0 { Some(project_id) } else { None },
            project_name: row.get(4)?,
            description: row.get(5)?,
            hours: row.get(6)?,
            date: row.get(7)?,
            source: "cache".into(),
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ── Attendance cache ────────────────────────────────────────────────

/// Replace all cached attendance blocks for a given date.
pub fn cache_attendance(conn: &Connection, date: &str, blocks: &[AttendanceBlock]) -> AppResult<()> {
    conn.execute("DELETE FROM cached_attendance WHERE date = ?1", params![date])?;
    let mut stmt = conn.prepare(
        "INSERT INTO cached_attendance (date, check_in, check_out, worked_hours)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for b in blocks {
        stmt.execute(params![date, b.check_in, b.check_out, b.worked_hours])?;
    }
    set_cache_updated(conn, &format!("att:{date}"))?;
    Ok(())
}

/// Read cached attendance blocks for a single date.
pub fn get_cached_attendance(conn: &Connection, date: &str) -> AppResult<Vec<AttendanceBlock>> {
    let mut stmt = conn.prepare(
        "SELECT check_in, check_out, worked_hours
         FROM cached_attendance
         WHERE date = ?1
         ORDER BY check_in",
    )?;
    let rows = stmt.query_map(params![date], |row| {
        Ok(AttendanceBlock {
            check_in: row.get(0)?,
            check_out: row.get(1)?,
            worked_hours: row.get(2)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ── Cache metadata ──────────────────────────────────────────────────

/// Mark a cache key as just-refreshed.
fn set_cache_updated(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO cache_meta (cache_key, updated_at)
         VALUES (?1, datetime('now'))",
        params![key],
    )?;
    Ok(())
}

/// Return the age of a cache key in seconds, or None if never cached.
pub fn get_cache_age_secs(conn: &Connection, key: &str) -> AppResult<Option<i64>> {
    let result: Option<i64> = conn
        .query_row(
            "SELECT CAST((julianday('now') - julianday(updated_at)) * 86400 AS INTEGER)
             FROM cache_meta WHERE cache_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok();
    Ok(result)
}

/// Check if dates in a range have cached data.
#[allow(dead_code)]
pub fn get_cached_dates_in_range(conn: &Connection, start: &str, end: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT date FROM cached_timesheet_entries
         WHERE date >= ?1 AND date <= ?2
         ORDER BY date",
    )?;
    let rows = stmt.query_map(params![start, end], |row| row.get(0))?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Bulk-cache timesheet entries for a range (used by monthly summary background sync).
pub fn cache_timesheet_entries_range(conn: &Connection, entries: &[TimesheetEntry]) -> AppResult<()> {
    // Group by date and cache each date
    let mut dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in entries {
        dates.insert(e.date.clone());
    }
    for date in &dates {
        let date_entries: Vec<&TimesheetEntry> = entries.iter().filter(|e| &e.date == date).collect();
        conn.execute("DELETE FROM cached_timesheet_entries WHERE date = ?1", params![date])?;
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO cached_timesheet_entries
             (odoo_id, task_id, task_name, project_id, project_name, description, hours, date)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for e in &date_entries {
            stmt.execute(params![
                e.id.unwrap_or(0),
                e.task_id.unwrap_or(0),
                e.task_name,
                e.project_id.unwrap_or(0),
                e.project_name,
                e.description,
                e.hours,
                e.date,
            ])?;
        }
        set_cache_updated(conn, &format!("ts:{date}"))?;
    }
    Ok(())
}
