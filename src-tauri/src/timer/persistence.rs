use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::AppResult;
use super::engine::{TimerEngine, TimerState};

/// Data recovered from the `timer_state` table.
#[derive(Debug, Clone, Serialize)]
pub struct SavedTimerState {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub start_utc: DateTime<Utc>,
    pub accumulated_secs: u64,
}

/// Ensure the project_id column exists (added after initial schema).
pub fn ensure_project_id_column(conn: &Connection) {
    let _ = conn.execute(
        "ALTER TABLE timer_state ADD COLUMN project_id INTEGER NOT NULL DEFAULT 0",
        [],
    );
}

/// Persist the current engine state into the single-row `timer_state` table.
pub fn save_timer_state(conn: &Connection, engine: &TimerEngine) -> AppResult<()> {
    match engine.raw_state() {
        TimerState::Idle => {
            clear_timer_state(conn)?;
        }
        TimerState::Running {
            task_id,
            task_name,
            project_id,
            project_name,
            start_time,
            accumulated_secs,
        } => {
            let start_str = start_time.to_rfc3339();
            conn.execute(
                "INSERT INTO timer_state (id, task_id, task_name, project_id, project_name, start_utc, accumulated_secs)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     task_id = excluded.task_id,
                     task_name = excluded.task_name,
                     project_id = excluded.project_id,
                     project_name = excluded.project_name,
                     start_utc = excluded.start_utc,
                     accumulated_secs = excluded.accumulated_secs",
                params![task_id, task_name, project_id, project_name, start_str, *accumulated_secs as i64],
            )?;
        }
    }
    Ok(())
}

/// Remove the persisted timer state (called on stop / discard).
pub fn clear_timer_state(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM timer_state", [])?;
    Ok(())
}

/// Attempt to load a previously saved timer state (crash recovery).
pub fn restore_timer_state(conn: &Connection) -> AppResult<Option<SavedTimerState>> {
    let result = conn
        .query_row(
            "SELECT task_id, task_name, project_id, project_name, start_utc, accumulated_secs
             FROM timer_state WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)? as u64,
                ))
            },
        )
        .optional()?;

    match result {
        None => Ok(None),
        Some((task_id, task_name, project_id, project_name, start_str, accumulated_secs)) => {
            let start_utc = DateTime::parse_from_rfc3339(&start_str)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| crate::error::AppError::General(format!(
                    "Failed to parse saved timer start_utc: {e}"
                )))?;
            Ok(Some(SavedTimerState {
                task_id,
                task_name,
                project_id,
                project_name,
                start_utc,
                accumulated_secs,
            }))
        }
    }
}
