//! Persistence for the cross-device live-session outbox.
//!
//! When a synced timer stops or is discarded, the matching Odoo write is queued
//! here rather than performed inline: Odoo can be slow or unreachable and the
//! UI must never wait on it. The reconciler drains these rows in the background.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppResult;

/// What still has to happen in Odoo for a session that is no longer running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxAction {
    /// Turn the live line into a normal timesheet entry (or create one).
    Finalize,
    /// Drop the live line entirely — the user discarded the run.
    Discard,
}

impl OutboxAction {
    fn as_str(self) -> &'static str {
        match self {
            OutboxAction::Finalize => "finalize",
            OutboxAction::Discard => "discard",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "discard" => OutboxAction::Discard,
            _ => OutboxAction::Finalize,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutboxEntry {
    pub session_id: String,
    pub odoo_line_id: Option<i64>,
    pub action: OutboxAction,
    pub task_id: i64,
    pub project_id: i64,
    pub task_name: String,
    pub project_name: String,
    pub description: String,
    pub hours: f64,
    pub date: String,
}

/// Queue (or replace) the pending Odoo write for a finished session.
pub fn enqueue(conn: &Connection, entry: &OutboxEntry) -> AppResult<()> {
    conn.execute(
        "INSERT INTO session_outbox
             (session_id, odoo_line_id, action, task_id, project_id,
              task_name, project_name, description, hours, date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(session_id) DO UPDATE SET
             odoo_line_id = excluded.odoo_line_id,
             action       = excluded.action,
             task_id      = excluded.task_id,
             project_id   = excluded.project_id,
             task_name    = excluded.task_name,
             project_name = excluded.project_name,
             description  = excluded.description,
             hours        = excluded.hours,
             date         = excluded.date",
        params![
            entry.session_id,
            entry.odoo_line_id,
            entry.action.as_str(),
            entry.task_id,
            entry.project_id,
            entry.task_name,
            entry.project_name,
            entry.description,
            entry.hours,
            entry.date,
        ],
    )?;
    Ok(())
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<OutboxEntry> {
    Ok(OutboxEntry {
        session_id: row.get(0)?,
        odoo_line_id: row.get(1)?,
        action: OutboxAction::parse(&row.get::<_, String>(2)?),
        task_id: row.get(3)?,
        project_id: row.get(4)?,
        task_name: row.get(5)?,
        project_name: row.get(6)?,
        description: row.get(7)?,
        hours: row.get(8)?,
        date: row.get(9)?,
    })
}

const SELECT_COLS: &str = "session_id, odoo_line_id, action, task_id, project_id,
                           task_name, project_name, description, hours, date";

pub fn list(conn: &Connection) -> AppResult<Vec<OutboxEntry>> {
    let sql = format!("SELECT {SELECT_COLS} FROM session_outbox ORDER BY created_at");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_entry)?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn get(conn: &Connection, session_id: &str) -> AppResult<Option<OutboxEntry>> {
    let sql = format!("SELECT {SELECT_COLS} FROM session_outbox WHERE session_id = ?1");
    Ok(conn
        .query_row(&sql, params![session_id], row_to_entry)
        .optional()?)
}

pub fn remove(conn: &Connection, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM session_outbox WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Attach the Odoo line that was created for a session after it already ended.
///
/// Happens when the user stops a timer while the initial `create` is still in
/// flight: the reconciler learns the line id only after the session left
/// `timer_state`, and the queued finalize would otherwise create a duplicate.
pub fn attach_line(conn: &Connection, session_id: &str, line_id: i64) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE session_outbox SET odoo_line_id = ?2 WHERE session_id = ?1 AND odoo_line_id IS NULL",
        params![session_id, line_id],
    )?;
    Ok(n > 0)
}

/// Amend a queued finalize with what the user actually typed in the log form.
pub fn amend_finalize(
    conn: &Connection,
    session_id: &str,
    description: &str,
    hours: f64,
    date: &str,
) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE session_outbox
            SET description = ?2, hours = ?3, date = ?4
          WHERE session_id = ?1 AND action = 'finalize'",
        params![session_id, description, hours, date],
    )?;
    Ok(n > 0)
}
