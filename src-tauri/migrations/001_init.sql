CREATE TABLE IF NOT EXISTS timer_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    task_id INTEGER NOT NULL,
    task_name TEXT NOT NULL,
    project_name TEXT NOT NULL DEFAULT '',
    start_utc TEXT NOT NULL,
    accumulated_secs INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cached_tasks (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    project_id INTEGER,
    project_name TEXT,
    stage_name TEXT,
    state TEXT,
    planned_hours REAL,
    effective_hours REAL,
    last_fetched TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cached_projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    last_fetched TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS recent_tasks (
    task_id INTEGER PRIMARY KEY,
    task_name TEXT NOT NULL,
    project_name TEXT,
    last_used TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_timesheets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    duration_hours REAL NOT NULL,
    date TEXT NOT NULL,
    created_at TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS timesheet_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    odoo_id INTEGER,
    task_id INTEGER NOT NULL,
    task_name TEXT NOT NULL,
    project_name TEXT,
    description TEXT NOT NULL DEFAULT '',
    duration_hours REAL NOT NULL,
    date TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_synced ON pending_timesheets(synced);
CREATE INDEX IF NOT EXISTS idx_recent_last_used ON recent_tasks(last_used DESC);
CREATE INDEX IF NOT EXISTS idx_timesheet_log_date ON timesheet_log(date);
CREATE INDEX IF NOT EXISTS idx_cached_tasks_project ON cached_tasks(project_id);
