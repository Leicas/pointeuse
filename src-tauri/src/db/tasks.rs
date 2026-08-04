use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

// ── Data types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTask {
    pub id: i64,
    pub name: String,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub stage_name: Option<String>,
    pub planned_hours: f64,
    pub effective_hours: f64,
    pub state: Option<String>,
    pub kanban_state: Option<String>,
    pub priority: Option<String>,
    pub date_deadline: Option<String>,
    pub write_date: Option<String>,
    pub create_date: Option<String>,
    pub user_ids_json: Option<String>, // JSON array of i64
    pub color: Option<i64>,
}

// ── TaskLike trait ───────────────────────────────────────────────────

pub trait TaskLike {
    fn task_id(&self) -> i64;
    fn task_name_ref(&self) -> &str;
    fn task_project_id(&self) -> Option<i64>;
    fn task_project_name_ref(&self) -> Option<&str>;
    fn task_stage_name_ref(&self) -> Option<&str> { None }
    fn task_planned_hours(&self) -> f64 { 0.0 }
    fn task_effective_hours(&self) -> f64 { 0.0 }
    fn task_state(&self) -> Option<&str> { None }
    fn task_kanban_state(&self) -> Option<&str> { None }
    fn task_priority(&self) -> Option<&str> { None }
    fn task_date_deadline(&self) -> Option<&str> { None }
    fn task_write_date(&self) -> Option<&str> { None }
    fn task_create_date(&self) -> Option<&str> { None }
    fn task_user_ids(&self) -> Option<&[i64]> { None }
    fn task_color(&self) -> Option<i64> { None }
}

impl TaskLike for CachedTask {
    fn task_id(&self) -> i64 { self.id }
    fn task_name_ref(&self) -> &str { &self.name }
    fn task_project_id(&self) -> Option<i64> { self.project_id }
    fn task_project_name_ref(&self) -> Option<&str> { self.project_name.as_deref() }
    fn task_stage_name_ref(&self) -> Option<&str> { self.stage_name.as_deref() }
    fn task_planned_hours(&self) -> f64 { self.planned_hours }
    fn task_effective_hours(&self) -> f64 { self.effective_hours }
    fn task_state(&self) -> Option<&str> { self.state.as_deref() }
    fn task_kanban_state(&self) -> Option<&str> { self.kanban_state.as_deref() }
    fn task_priority(&self) -> Option<&str> { self.priority.as_deref() }
    fn task_date_deadline(&self) -> Option<&str> { self.date_deadline.as_deref() }
    fn task_write_date(&self) -> Option<&str> { self.write_date.as_deref() }
    fn task_create_date(&self) -> Option<&str> { self.create_date.as_deref() }
    fn task_color(&self) -> Option<i64> { self.color }
}

// ── Task cache ───────────────────────────────────────────────────────

const INSERT_SQL: &str =
    "INSERT INTO cached_tasks (id, name, project_id, project_name, stage_name,
        planned_hours, effective_hours, is_my_task, state, kanban_state, priority,
        date_deadline, write_date, create_date, user_ids, color)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
     ON CONFLICT(id) DO UPDATE SET
         name            = excluded.name,
         project_id      = excluded.project_id,
         project_name    = excluded.project_name,
         stage_name      = excluded.stage_name,
         planned_hours   = excluded.planned_hours,
         effective_hours = excluded.effective_hours,
         is_my_task      = CASE WHEN ?8 = 1 THEN 1 ELSE cached_tasks.is_my_task END,
         state           = excluded.state,
         kanban_state    = excluded.kanban_state,
         priority        = excluded.priority,
         date_deadline   = excluded.date_deadline,
         write_date      = excluded.write_date,
         create_date     = excluded.create_date,
         user_ids        = excluded.user_ids,
         color           = excluded.color";

const SELECT_COLS: &str =
    "id, name, project_id, project_name, stage_name, planned_hours, effective_hours,
     state, kanban_state, priority, date_deadline, write_date, create_date, user_ids, color";

fn row_to_cached_task(row: &rusqlite::Row) -> rusqlite::Result<CachedTask> {
    Ok(CachedTask {
        id: row.get(0)?,
        name: row.get(1)?,
        project_id: row.get(2)?,
        project_name: row.get(3)?,
        stage_name: row.get(4)?,
        planned_hours: row.get(5)?,
        effective_hours: row.get(6)?,
        state: row.get(7)?,
        kanban_state: row.get(8)?,
        priority: row.get(9)?,
        date_deadline: row.get(10)?,
        write_date: row.get(11)?,
        create_date: row.get(12)?,
        user_ids_json: row.get(13)?,
        color: row.get(14)?,
    })
}

/// Serialize user_ids to JSON for storage.
fn user_ids_to_json(t: &dyn TaskLike) -> Option<String> {
    t.task_user_ids().map(|ids| serde_json::to_string(ids).unwrap_or_default())
}

/// Upsert a batch of tasks into the local cache.
pub fn cache_tasks_with_flag<T: TaskLike>(conn: &Connection, tasks: &[T], my_tasks: bool) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        if my_tasks {
            tx.execute("UPDATE cached_tasks SET is_my_task = 0", [])?;
        }
        let mut stmt = tx.prepare(INSERT_SQL)?;
        let flag: i64 = if my_tasks { 1 } else { 0 };
        for t in tasks {
            stmt.execute(params![
                t.task_id(),
                t.task_name_ref(),
                t.task_project_id(),
                t.task_project_name_ref().unwrap_or(""),
                t.task_stage_name_ref(),
                t.task_planned_hours(),
                t.task_effective_hours(),
                flag,
                t.task_state(),
                t.task_kanban_state(),
                t.task_priority(),
                t.task_date_deadline(),
                t.task_write_date(),
                t.task_create_date(),
                user_ids_to_json(t),
                t.task_color(),
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Upsert a batch of tasks (backward-compatible, no flag change).
pub fn cache_tasks<T: TaskLike>(conn: &Connection, tasks: &[T]) -> AppResult<()> {
    cache_tasks_with_flag(conn, tasks, false)
}

/// Read cached tasks marked as "my tasks".
pub fn get_cached_my_tasks(conn: &Connection) -> AppResult<Vec<CachedTask>> {
    let sql = format!("SELECT {SELECT_COLS} FROM cached_tasks WHERE is_my_task = 1 ORDER BY name ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_cached_task)?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Search the cached tasks by name and optionally project.
pub fn get_cached_tasks<Q: AsRef<str>>(
    conn: &Connection,
    query: Q,
    project_id: Option<i64>,
) -> AppResult<Vec<CachedTask>> {
    let q = query.as_ref();
    let mut sql = format!("SELECT {SELECT_COLS} FROM cached_tasks WHERE 1=1");
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if !q.is_empty() {
        sql.push_str(" AND name LIKE ?");
        bind_values.push(Box::new(format!("%{q}%")));
    }
    if let Some(pid) = project_id {
        sql.push_str(" AND project_id = ?");
        bind_values.push(Box::new(pid));
    }
    sql.push_str(" ORDER BY name ASC");

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        bind_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), row_to_cached_task)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ── Recent tasks ─────────────────────────────────────────────────────

/// Record (or bump) a task in the recent-tasks list.
pub fn touch_recent(
    conn: &Connection,
    task_id: i64,
    task_name: &str,
    project_name: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO recent_tasks (task_id, task_name, project_name, last_used)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(task_id) DO UPDATE SET
             task_name    = excluded.task_name,
             project_name = excluded.project_name,
             last_used    = excluded.last_used",
        params![task_id, task_name, project_name.unwrap_or("")],
    )?;
    Ok(())
}

/// Record a task as used at an explicit time, keeping whichever stamp is later.
///
/// Used when seeding recents from another device's Odoo timesheet history: a
/// day-old remote entry must not overwrite a task this device used minutes ago.
pub fn touch_recent_at(
    conn: &Connection,
    task_id: i64,
    task_name: &str,
    project_name: Option<&str>,
    used_at: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO recent_tasks (task_id, task_name, project_name, last_used)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(task_id) DO UPDATE SET
             task_name    = excluded.task_name,
             project_name = excluded.project_name,
             last_used    = MAX(recent_tasks.last_used, excluded.last_used)",
        params![task_id, task_name, project_name.unwrap_or(""), used_at],
    )?;
    Ok(())
}

/// Return the most recently used tasks, enriched with cached metadata.
pub fn get_recent_tasks(conn: &Connection, limit: usize) -> AppResult<Vec<CachedTask>> {
    let mut stmt = conn.prepare(
        "SELECT r.task_id, r.task_name, c.project_id, r.project_name,
                c.stage_name, COALESCE(c.planned_hours, 0), COALESCE(c.effective_hours, 0),
                c.state, c.kanban_state, c.priority, c.date_deadline,
                c.write_date, c.create_date, c.user_ids, c.color
         FROM recent_tasks r
         LEFT JOIN cached_tasks c ON c.id = r.task_id
         ORDER BY r.last_used DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], row_to_cached_task)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
