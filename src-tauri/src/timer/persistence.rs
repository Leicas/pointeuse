use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::AppResult;
use super::engine::{SessionMeta, TimerEngine, TimerState};

/// Data recovered from the `timer_state` table.
#[derive(Debug, Clone, Serialize)]
pub struct SavedTimerState {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub start_utc: DateTime<Utc>,
    pub accumulated_secs: u64,
    pub session: SessionMeta,
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
            session,
        } => {
            let start_str = start_time.to_rfc3339();
            conn.execute(
                "INSERT INTO timer_state (id, task_id, task_name, project_id, project_name, start_utc,
                                          accumulated_secs, session_id, origin_device, origin_label, odoo_line_id)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                     task_id = excluded.task_id,
                     task_name = excluded.task_name,
                     project_id = excluded.project_id,
                     project_name = excluded.project_name,
                     start_utc = excluded.start_utc,
                     accumulated_secs = excluded.accumulated_secs,
                     session_id = excluded.session_id,
                     origin_device = excluded.origin_device,
                     origin_label = excluded.origin_label,
                     odoo_line_id = excluded.odoo_line_id",
                params![
                    task_id,
                    task_name,
                    project_id,
                    project_name,
                    start_str,
                    *accumulated_secs as i64,
                    session.session_id,
                    session.origin_device,
                    session.origin_label,
                    session.odoo_line_id,
                ],
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
            "SELECT task_id, task_name, project_id, project_name, start_utc, accumulated_secs,
                    COALESCE(session_id, ''), COALESCE(origin_device, ''), COALESCE(origin_label, ''), odoo_line_id
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
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                ))
            },
        )
        .optional()?;

    match result {
        None => Ok(None),
        Some((
            task_id,
            task_name,
            project_id,
            project_name,
            start_str,
            accumulated_secs,
            session_id,
            origin_device,
            origin_label,
            odoo_line_id,
        )) => {
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
                session: SessionMeta {
                    session_id,
                    origin_device,
                    origin_label,
                    odoo_line_id,
                },
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::engine::SessionMeta;

    fn scratch_db(tag: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("pointeuse-persist-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let conn = crate::db::schema::initialize_database(&dir).unwrap();
        ensure_project_id_column(&conn);
        (dir, conn)
    }

    /// Guards the column list in `save_timer_state` against the migration:
    /// a mismatch there would only blow up on a real user's running timer.
    #[test]
    fn a_synced_run_survives_a_restart() {
        let (dir, conn) = scratch_db("synced");
        let start = Utc::now() - chrono::Duration::seconds(90);

        let mut engine = TimerEngine::new();
        engine.restore(
            7,
            "Wire up the sync".into(),
            3,
            "Pointeuse".into(),
            start,
            0,
            SessionMeta {
                session_id: "sess-1".into(),
                origin_device: "dev-a".into(),
                origin_label: "Pixel 8".into(),
                odoo_line_id: Some(4242),
            },
        );
        save_timer_state(&conn, &engine).unwrap();

        let saved = restore_timer_state(&conn).unwrap().expect("a saved run");
        assert_eq!(saved.task_id, 7);
        assert_eq!(saved.project_id, 3);
        assert_eq!(saved.session.session_id, "sess-1");
        assert_eq!(saved.session.origin_device, "dev-a");
        assert_eq!(saved.session.origin_label, "Pixel 8");
        assert_eq!(saved.session.odoo_line_id, Some(4242));
        assert_eq!(saved.start_utc.timestamp(), start.timestamp());

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unsynced_run_round_trips_with_an_empty_session() {
        let (dir, conn) = scratch_db("local");
        let mut engine = TimerEngine::new();
        engine
            .start(1, "Local only".into(), 2, "Proj".into(), SessionMeta::local_only())
            .unwrap();
        save_timer_state(&conn, &engine).unwrap();

        let saved = restore_timer_state(&conn).unwrap().expect("a saved run");
        assert!(saved.session.session_id.is_empty());
        assert_eq!(saved.session.odoo_line_id, None);

        clear_timer_state(&conn).unwrap();
        assert!(restore_timer_state(&conn).unwrap().is_none());

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
