use std::path::Path;

use log::info;
use rusqlite::Connection;

use crate::error::AppResult;

/// Full initial schema.  Kept as a const so the binary is self-contained
/// (no need to ship a separate .sql file).
const MIGRATION_001_INIT: &str = r#"
-- Tasks cache (synced from Odoo)
CREATE TABLE IF NOT EXISTS cached_tasks (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    project_id   INTEGER,
    project_name TEXT NOT NULL DEFAULT ''
);

-- Projects cache
CREATE TABLE IF NOT EXISTS cached_projects (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

-- Recently-used tasks (local only)
CREATE TABLE IF NOT EXISTS recent_tasks (
    task_id      INTEGER PRIMARY KEY,
    task_name    TEXT NOT NULL,
    project_name TEXT NOT NULL DEFAULT '',
    last_used    TEXT NOT NULL
);

-- Pending timesheets waiting to be synced
CREATE TABLE IF NOT EXISTS pending_timesheets (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id        INTEGER NOT NULL,
    project_id     INTEGER NOT NULL DEFAULT 0,
    description    TEXT NOT NULL DEFAULT '',
    duration_hours REAL NOT NULL,
    date           TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Local log of synced timesheets
CREATE TABLE IF NOT EXISTS timesheet_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id      INTEGER NOT NULL,
    task_name    TEXT NOT NULL,
    project_name TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    hours        REAL NOT NULL,
    date         TEXT NOT NULL,
    odoo_id      INTEGER,
    synced_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Running-timer crash-recovery row (at most one row, id always = 1)
CREATE TABLE IF NOT EXISTS timer_state (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    task_id          INTEGER NOT NULL,
    task_name        TEXT NOT NULL,
    project_name     TEXT NOT NULL DEFAULT '',
    start_utc        TEXT NOT NULL,
    accumulated_secs INTEGER NOT NULL DEFAULT 0
);
"#;

/// Migration 002: Odoo data cache tables for instant UI responses.
const MIGRATION_002_CACHE: &str = r#"
-- Cached timesheet entries from Odoo (keyed by Odoo id)
CREATE TABLE IF NOT EXISTS cached_timesheet_entries (
    odoo_id      INTEGER NOT NULL,
    task_id      INTEGER,
    task_name    TEXT NOT NULL DEFAULT '',
    project_id   INTEGER,
    project_name TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    hours        REAL NOT NULL,
    date         TEXT NOT NULL,
    PRIMARY KEY (odoo_id)
);
CREATE INDEX IF NOT EXISTS idx_cached_ts_date ON cached_timesheet_entries(date);

-- Cached attendance blocks from Odoo
CREATE TABLE IF NOT EXISTS cached_attendance (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    date         TEXT NOT NULL,
    check_in     TEXT NOT NULL,
    check_out    TEXT,
    worked_hours REAL NOT NULL DEFAULT 0.0
);
CREATE INDEX IF NOT EXISTS idx_cached_att_date ON cached_attendance(date);

-- Tracks when each cache key was last refreshed from Odoo
CREATE TABLE IF NOT EXISTS cache_meta (
    cache_key  TEXT PRIMARY KEY,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// Migration 007: outbox of pending Odoo writes for finished live sessions.
///
/// A run that was published to Odoo owns an `account.analytic.line`; stopping
/// or discarding it has to reach Odoo eventually, even if the app is offline or
/// gets killed in between. Rows live here until the reconciler flushes them.
const MIGRATION_007_SESSION_OUTBOX: &str = r#"
CREATE TABLE IF NOT EXISTS session_outbox (
    session_id   TEXT PRIMARY KEY,
    odoo_line_id INTEGER,
    action       TEXT NOT NULL,           -- 'finalize' | 'discard'
    task_id      INTEGER NOT NULL,
    project_id   INTEGER NOT NULL DEFAULT 0,
    task_name    TEXT NOT NULL DEFAULT '',
    project_name TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    hours        REAL NOT NULL DEFAULT 0.0,
    date         TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// Open (or create) the database at `<app_data_dir>/timetracker.db` and run
/// all pending migrations.
pub fn initialize_database(app_data_dir: &Path) -> AppResult<Connection> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| {
        crate::error::AppError::General(format!(
            "Failed to create app data directory: {e}"
        ))
    })?;

    let db_path = app_data_dir.join("timetracker.db");
    info!("Opening database at {}", db_path.display());

    let conn = Connection::open(&db_path)?;

    // Basic pragmas for performance & safety
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;

    conn.execute_batch(MIGRATION_001_INIT)?;
    conn.execute_batch(MIGRATION_002_CACHE)?;

    // Migration 003: Add is_my_task + extra columns to cached_tasks
    let has_is_my_task: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('cached_tasks') WHERE name='is_my_task'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .unwrap_or(0)
        > 0;
    if !has_is_my_task {
        conn.execute_batch(
            "ALTER TABLE cached_tasks ADD COLUMN is_my_task INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE cached_tasks ADD COLUMN stage_name TEXT;
             ALTER TABLE cached_tasks ADD COLUMN planned_hours REAL NOT NULL DEFAULT 0.0;
             ALTER TABLE cached_tasks ADD COLUMN effective_hours REAL NOT NULL DEFAULT 0.0;",
        )?;
    }

    // Migration 004: Add state, kanban_state, priority, dates, user_ids, color
    let has_state: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('cached_tasks') WHERE name='state'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .unwrap_or(0)
        > 0;
    if !has_state {
        conn.execute_batch(
            "ALTER TABLE cached_tasks ADD COLUMN state TEXT;
             ALTER TABLE cached_tasks ADD COLUMN kanban_state TEXT;
             ALTER TABLE cached_tasks ADD COLUMN priority TEXT;
             ALTER TABLE cached_tasks ADD COLUMN date_deadline TEXT;
             ALTER TABLE cached_tasks ADD COLUMN write_date TEXT;
             ALTER TABLE cached_tasks ADD COLUMN create_date TEXT;
             ALTER TABLE cached_tasks ADD COLUMN user_ids TEXT;
             ALTER TABLE cached_tasks ADD COLUMN color INTEGER;",
        )?;
    }

    // Migration 005: Enhanced pending_timesheets for robust sync
    let has_status: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('pending_timesheets') WHERE name='status'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .unwrap_or(0)
        > 0;
    if !has_status {
        conn.execute_batch(
            "ALTER TABLE pending_timesheets ADD COLUMN status TEXT NOT NULL DEFAULT 'pending';
             ALTER TABLE pending_timesheets ADD COLUMN odoo_id INTEGER;
             ALTER TABLE pending_timesheets ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE pending_timesheets ADD COLUMN last_error TEXT;
             ALTER TABLE pending_timesheets ADD COLUMN last_attempt_at TEXT;
             CREATE INDEX IF NOT EXISTS idx_pending_status ON pending_timesheets(status);",
        )?;
    }

    // Migration 006: Manual timesheet entries — duplicate bypass + denormalised names
    let has_allow_duplicate: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('pending_timesheets') WHERE name='allow_duplicate'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .unwrap_or(0)
        > 0;
    if !has_allow_duplicate {
        conn.execute_batch(
            "ALTER TABLE pending_timesheets ADD COLUMN allow_duplicate INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE pending_timesheets ADD COLUMN task_name TEXT NOT NULL DEFAULT '';
             ALTER TABLE pending_timesheets ADD COLUMN project_name TEXT NOT NULL DEFAULT '';",
        )?;
    }

    // Migration 007: Cross-device timer sync — session identity on the running
    // timer plus an outbox of Odoo writes that must survive an app restart.
    let has_session_id: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('timer_state') WHERE name='session_id'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .unwrap_or(0)
        > 0;
    if !has_session_id {
        conn.execute_batch(
            "ALTER TABLE timer_state ADD COLUMN session_id TEXT NOT NULL DEFAULT '';
             ALTER TABLE timer_state ADD COLUMN origin_device TEXT NOT NULL DEFAULT '';
             ALTER TABLE timer_state ADD COLUMN origin_label TEXT NOT NULL DEFAULT '';
             ALTER TABLE timer_state ADD COLUMN odoo_line_id INTEGER;",
        )?;
    }
    conn.execute_batch(MIGRATION_007_SESSION_OUTBOX)?;

    info!("Database migrations applied successfully");

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pointeuse-schema-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .and_then(|mut s| {
                s.query_map([], |r| r.get::<_, String>(0))
                    .map(|rows| rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default()
    }

    #[test]
    fn fresh_database_has_the_live_session_schema() {
        let dir = scratch_dir("fresh");
        let conn = initialize_database(&dir).expect("migrations should apply");

        let timer_cols = columns(&conn, "timer_state");
        for expected in ["session_id", "origin_device", "origin_label", "odoo_line_id"] {
            assert!(timer_cols.contains(&expected.to_string()), "missing {expected}");
        }
        assert!(columns(&conn, "session_outbox").contains(&"action".to_string()));

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The upgrade path that matters: a database created before cross-device
    /// sync existed must gain the new columns without losing its running timer.
    #[test]
    fn upgrades_a_pre_sync_database_in_place() {
        let dir = scratch_dir("upgrade");
        std::fs::create_dir_all(&dir).unwrap();

        {
            let old = Connection::open(dir.join("timetracker.db")).unwrap();
            old.execute_batch(
                "CREATE TABLE timer_state (
                     id               INTEGER PRIMARY KEY CHECK (id = 1),
                     task_id          INTEGER NOT NULL,
                     task_name        TEXT NOT NULL,
                     project_name     TEXT NOT NULL DEFAULT '',
                     start_utc        TEXT NOT NULL,
                     accumulated_secs INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO timer_state (id, task_id, task_name, start_utc)
                 VALUES (1, 42, 'Existing run', '2026-08-04T09:00:00+00:00');",
            )
            .unwrap();
        }

        let conn = initialize_database(&dir).expect("migrations should apply");
        crate::timer::persistence::ensure_project_id_column(&conn);

        let timer_cols = columns(&conn, "timer_state");
        for expected in ["project_id", "session_id", "origin_device", "origin_label", "odoo_line_id"] {
            assert!(timer_cols.contains(&expected.to_string()), "missing {expected}");
        }

        // The in-flight run survives, defaulting to "not synced".
        let (task_id, session_id): (i64, String) = conn
            .query_row(
                "SELECT task_id, session_id FROM timer_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(task_id, 42);
        assert_eq!(session_id, "");

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
