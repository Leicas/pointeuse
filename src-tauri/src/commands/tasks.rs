use std::collections::HashMap;

use serde::Serialize;

use crate::db::cache::get_cache_age_secs;
use crate::db::tasks::{
    cache_tasks, cache_tasks_with_flag, get_cached_my_tasks, get_cached_tasks,
    get_recent_tasks as db_get_recent_tasks, TaskLike,
};
use crate::db::timesheets::get_task_ranking;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub id: i64,
    pub name: String,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub stage_id: Option<i64>,
    pub stage_name: Option<String>,
    pub state: Option<String>,
    pub kanban_state: Option<String>,
    pub planned_hours: f64,
    pub effective_hours: f64,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub date_deadline: Option<String>,
    pub write_date: Option<String>,
    pub create_date: Option<String>,
    pub user_ids: Option<Vec<i64>>,
    pub parent_id: Option<(i64, String)>,
    pub color: Option<i64>,
}

impl TaskLike for TaskInfo {
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
    fn task_user_ids(&self) -> Option<&[i64]> { self.user_ids.as_deref() }
    fn task_color(&self) -> Option<i64> { self.color }
}

fn odoo_task_to_info(t: crate::odoo::models::OdooTask) -> TaskInfo {
    TaskInfo {
        id: t.id,
        name: t.name,
        project_id: t.project_id.as_ref().map(|(id, _)| *id),
        project_name: t.project_id.map(|(_, name)| name),
        stage_id: t.stage_id.as_ref().map(|(id, _)| *id),
        stage_name: t.stage_name,
        state: t.state,
        kanban_state: t.kanban_state,
        planned_hours: t.planned_hours,
        effective_hours: t.effective_hours,
        description: t.description,
        priority: t.priority,
        date_deadline: t.date_deadline,
        write_date: t.write_date,
        create_date: t.create_date,
        user_ids: t.user_ids,
        parent_id: t.parent_id,
        color: t.color,
    }
}

#[tauri::command]
pub async fn search_tasks(
    query: String,
    project_id: Option<i64>,
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<TaskInfo>> {
    log::info!("search_tasks: query='{}', project_id={:?}", query, project_id);

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.search_tasks(&query, project_id).await {
            Ok(tasks) => {
                log::info!("search_tasks: got {} results from Odoo", tasks.len());
                let task_infos: Vec<TaskInfo> = tasks.into_iter().map(odoo_task_to_info).collect();

                if let Ok(db) = state.db.lock() {
                    if let Err(e) = cache_tasks(&db, &task_infos) {
                        log::error!("search_tasks: failed to cache: {e}");
                    }
                }

                return Ok(task_infos);
            }
            Err(e) => {
                log::error!("search_tasks: Odoo error, falling back to cache: {e}");
            }
        }
    }

    let db = state.db.lock().unwrap();
    let cached = get_cached_tasks(&db, &query, project_id)?;
    log::info!("search_tasks: returning {} cached results", cached.len());
    Ok(cached.into_iter().map(cached_task_to_info).collect())
}

/// Cache TTL in seconds — serve cached data if fresher than this
const TASK_CACHE_TTL_SECS: i64 = 120;

/// Cache key used in cache_meta table for my_tasks
const MY_TASKS_CACHE_KEY: &str = "my_tasks";
/// Cache key used in cache_meta table for all_tasks
const ALL_TASKS_CACHE_KEY: &str = "all_tasks";

fn cached_task_to_info(c: crate::db::tasks::CachedTask) -> TaskInfo {
    let user_ids = c.user_ids_json.as_deref()
        .and_then(|s| serde_json::from_str::<Vec<i64>>(s).ok());
    TaskInfo {
        id: c.id,
        name: c.name,
        project_id: c.project_id,
        project_name: c.project_name,
        stage_id: None,
        stage_name: c.stage_name,
        state: c.state,
        kanban_state: c.kanban_state,
        planned_hours: c.planned_hours,
        effective_hours: c.effective_hours,
        description: None,
        priority: c.priority,
        date_deadline: c.date_deadline,
        write_date: c.write_date,
        create_date: c.create_date,
        user_ids,
        parent_id: None,
        color: c.color,
    }
}

#[tauri::command]
pub async fn get_my_tasks(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<Vec<TaskInfo>> {
    // 1. Check SQLite cache first
    let (cached_data, cache_age) = {
        let db = state.db.lock().unwrap();
        let cached = get_cached_my_tasks(&db).unwrap_or_default();
        let age = get_cache_age_secs(&db, MY_TASKS_CACHE_KEY).unwrap_or(None);
        (cached, age)
    };

    if !cached_data.is_empty() {
        let is_stale = cache_age.map_or(true, |age| age > TASK_CACHE_TTL_SECS);
        let task_infos: Vec<TaskInfo> = cached_data.into_iter().map(cached_task_to_info).collect();

        if is_stale {
            log::info!("get_my_tasks: returning {} stale cached tasks, refreshing in background", task_infos.len());
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                refresh_my_tasks_background(handle).await;
            });
        } else {
            log::info!("get_my_tasks: returning {} tasks from fresh SQLite cache", task_infos.len());
        }

        // Also update in-memory cache for other callers
        {
            let mut cache = state.tasks_cache.lock().unwrap();
            *cache = Some(crate::state::CacheEntry::new(task_infos.clone()));
        }
        return Ok(task_infos);
    }

    // 2. Check in-memory cache (fallback during first run before SQLite populated)
    let mem_cached = {
        let cache = state.tasks_cache.lock().unwrap();
        cache.as_ref().map(|e| e.data.clone())
    };
    if let Some(data) = mem_cached {
        log::info!("get_my_tasks: returning {} tasks from in-memory cache, refreshing", data.len());
        let handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            refresh_my_tasks_background(handle).await;
        });
        return Ok(data);
    }

    // 3. No cache at all — must fetch synchronously
    log::info!("get_my_tasks: no cache, fetching from Odoo");
    let task_infos = fetch_my_tasks_from_odoo(&state).await?;

    // Store in caches
    {
        let mut cache = state.tasks_cache.lock().unwrap();
        *cache = Some(crate::state::CacheEntry::new(task_infos.clone()));
    }

    Ok(task_infos)
}

async fn fetch_my_tasks_from_odoo(state: &AppState) -> AppResult<Vec<TaskInfo>> {
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let client = odoo_client.ok_or_else(|| {
        log::error!("get_my_tasks: not authenticated");
        AppError::Auth("Not authenticated".to_string())
    })?;
    let uid = client.uid();

    let tasks = client.get_my_tasks(uid).await.map_err(|e| {
        log::error!("get_my_tasks: Odoo error: {e}");
        AppError::Odoo(format!("Failed to fetch my tasks: {e}"))
    })?;

    log::info!("get_my_tasks: got {} tasks from Odoo", tasks.len());
    let task_infos: Vec<TaskInfo> = tasks.into_iter().map(odoo_task_to_info).collect();

    if let Ok(db) = state.db.lock() {
        if let Err(e) = cache_tasks_with_flag(&db, &task_infos, true) {
            log::error!("get_my_tasks: failed to cache: {e}");
        }
        // Update cache_meta timestamp
        let _ = db.execute(
            "INSERT OR REPLACE INTO cache_meta (cache_key, updated_at) VALUES (?1, datetime('now'))",
            rusqlite::params![MY_TASKS_CACHE_KEY],
        );
    }

    Ok(task_infos)
}

async fn refresh_my_tasks_background(app_handle: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let _ = app_handle.emit("cache_sync_start", "my_tasks");
    let state = app_handle.state::<AppState>();
    match fetch_my_tasks_from_odoo(&state).await {
        Ok(task_infos) => {
            log::info!("get_my_tasks background refresh: got {} tasks", task_infos.len());
            {
                let mut cache = state.tasks_cache.lock().unwrap();
                *cache = Some(crate::state::CacheEntry::new(task_infos.clone()));
            }
            // Notify frontend that fresh data is available
            let _ = app_handle.emit("tasks_refreshed", &task_infos);
        }
        Err(e) => {
            log::error!("get_my_tasks background refresh failed: {e}");
        }
    }
    let _ = app_handle.emit("cache_sync_done", "my_tasks");
}

#[tauri::command]
pub async fn get_recent_tasks(state: tauri::State<'_, AppState>) -> AppResult<Vec<TaskInfo>> {
    log::info!("get_recent_tasks: fetching");
    let db = state.db.lock().unwrap();
    let tasks = db_get_recent_tasks(&db, 20)?;
    log::info!("get_recent_tasks: returning {} tasks", tasks.len());
    Ok(tasks.into_iter().map(cached_task_to_info).collect())
}

#[tauri::command]
pub async fn create_task(
    name: String,
    project_id: i64,
    state: tauri::State<'_, AppState>,
) -> AppResult<TaskInfo> {
    log::info!("create_task: '{}' in project {}", name, project_id);

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let client = odoo_client.ok_or_else(|| {
        log::error!("create_task: not authenticated");
        AppError::Auth("Not authenticated".to_string())
    })?;

    match client.create_task(&name, project_id).await {
        Ok(task) => {
            log::info!("create_task: created task id={}", task.id);
            Ok(odoo_task_to_info(task))
        }
        Err(e) => {
            log::error!("create_task: Odoo error: {e}");
            Err(AppError::Odoo(format!("Failed to create task: {e}")))
        }
    }
}

// ── All Tasks (team-wide) ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub name: String,
}

#[tauri::command]
pub async fn get_all_tasks(
    project_ids: Vec<i64>,
    user_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<Vec<TaskInfo>> {
    log::info!(
        "get_all_tasks: project_ids={:?}, user_ids={:?}",
        project_ids, user_ids
    );

    // 1. Check SQLite cache first
    let (cached_data, cache_age) = {
        let db = state.db.lock().unwrap();
        let cached = get_cached_tasks(&db, "", None).unwrap_or_default();
        let age = get_cache_age_secs(&db, ALL_TASKS_CACHE_KEY).unwrap_or(None);
        (cached, age)
    };

    if !cached_data.is_empty() {
        let is_stale = cache_age.map_or(true, |age| age > TASK_CACHE_TTL_SECS);
        let task_infos: Vec<TaskInfo> = cached_data.into_iter().map(cached_task_to_info).collect();

        if is_stale {
            log::info!("get_all_tasks: returning {} stale cached tasks, refreshing in background", task_infos.len());
            let handle = app_handle.clone();
            let p_ids = project_ids.clone();
            let u_ids = user_ids.clone();
            tauri::async_runtime::spawn(async move {
                refresh_all_tasks_background(handle, p_ids, u_ids).await;
            });
        } else {
            log::info!("get_all_tasks: returning {} tasks from fresh SQLite cache", task_infos.len());
        }
        return Ok(task_infos);
    }

    // 2. No cache — fetch synchronously from Odoo
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.get_all_tasks(&project_ids, &user_ids).await {
            Ok(tasks) => {
                log::info!("get_all_tasks: got {} tasks from Odoo", tasks.len());
                let task_infos: Vec<TaskInfo> =
                    tasks.into_iter().map(odoo_task_to_info).collect();

                if let Ok(db) = state.db.lock() {
                    if let Err(e) = cache_tasks(&db, &task_infos) {
                        log::error!("get_all_tasks: failed to cache: {e}");
                    }
                    let _ = db.execute(
                        "INSERT OR REPLACE INTO cache_meta (cache_key, updated_at) VALUES (?1, datetime('now'))",
                        rusqlite::params![ALL_TASKS_CACHE_KEY],
                    );
                }

                return Ok(task_infos);
            }
            Err(e) => {
                log::error!("get_all_tasks: Odoo error, falling back to cache: {e}");
            }
        }
    }

    // Fallback: return cached tasks (unfiltered)
    let db = state.db.lock().unwrap();
    let cached = get_cached_tasks(&db, "", None)?;
    log::info!("get_all_tasks: returning {} cached results", cached.len());
    Ok(cached.into_iter().map(cached_task_to_info).collect())
}

async fn fetch_all_tasks_from_odoo(state: &AppState, project_ids: &[i64], user_ids: &[i64]) -> AppResult<Vec<TaskInfo>> {
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let client = odoo_client.ok_or_else(|| {
        AppError::Auth("Not authenticated".to_string())
    })?;

    let tasks = client.get_all_tasks(project_ids, user_ids).await.map_err(|e| {
        AppError::Odoo(format!("Failed to fetch all tasks: {e}"))
    })?;

    let task_infos: Vec<TaskInfo> = tasks.into_iter().map(odoo_task_to_info).collect();

    if let Ok(db) = state.db.lock() {
        if let Err(e) = cache_tasks(&db, &task_infos) {
            log::error!("get_all_tasks: failed to cache: {e}");
        }
        let _ = db.execute(
            "INSERT OR REPLACE INTO cache_meta (cache_key, updated_at) VALUES (?1, datetime('now'))",
            rusqlite::params![ALL_TASKS_CACHE_KEY],
        );
    }

    Ok(task_infos)
}

async fn refresh_all_tasks_background(app_handle: tauri::AppHandle, project_ids: Vec<i64>, user_ids: Vec<i64>) {
    use tauri::{Emitter, Manager};
    let _ = app_handle.emit("cache_sync_start", "all_tasks");
    let state = app_handle.state::<AppState>();
    match fetch_all_tasks_from_odoo(&state, &project_ids, &user_ids).await {
        Ok(task_infos) => {
            log::info!("get_all_tasks background refresh: got {} tasks", task_infos.len());
            let _ = app_handle.emit("all_tasks_refreshed", &task_infos);
        }
        Err(e) => {
            log::error!("get_all_tasks background refresh failed: {e}");
        }
    }
    let _ = app_handle.emit("cache_sync_done", "all_tasks");
}

#[tauri::command]
pub async fn get_all_users(
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<UserInfo>> {
    log::info!("get_all_users: fetching internal users");

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let client = odoo_client.ok_or_else(|| {
        log::error!("get_all_users: not authenticated");
        AppError::Auth("Not authenticated".to_string())
    })?;

    let users = client.get_all_users().await.map_err(|e| {
        log::error!("get_all_users: Odoo error: {e}");
        AppError::Odoo(format!("Failed to fetch users: {e}"))
    })?;

    log::info!("get_all_users: got {} users", users.len());
    Ok(users
        .into_iter()
        .map(|(id, name)| UserInfo { id, name })
        .collect())
}

// ── Smart task suggestions ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SuggestedTask {
    // Flat fields for compatibility with renderTaskItem
    pub id: i64,
    pub name: String,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub stage_id: Option<i64>,
    pub stage_name: Option<String>,
    pub state: Option<String>,
    pub kanban_state: Option<String>,
    pub planned_hours: f64,
    pub effective_hours: f64,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub date_deadline: Option<String>,
    pub write_date: Option<String>,
    pub create_date: Option<String>,
    pub user_ids: Option<Vec<i64>>,
    pub parent_id: Option<(i64, String)>,
    pub color: Option<i64>,
    // Extra scoring fields
    pub score: f64,
    pub reason: Option<String>,
}

#[tauri::command]
pub async fn get_suggested_tasks(
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<SuggestedTask>> {
    log::info!("get_suggested_tasks: computing smart ranking");

    let today = chrono::Local::now();
    let today_str = today.format("%Y-%m-%d").to_string();
    let weekday = today.format("%w").to_string().parse::<u32>().unwrap_or(0);

    // Get ranking data from local history
    let ranking_data = {
        let db = state.db.lock().unwrap();
        get_task_ranking(&db, &today_str, weekday).unwrap_or_default()
    };

    // Build a score map from historical data
    let mut score_map: HashMap<i64, (f64, String)> = HashMap::new();
    let max_freq = ranking_data
        .iter()
        .map(|r| r.frequency_30d)
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    for r in &ranking_data {
        let recency_score = 1.0 / ((1.0 + 0.15 * r.days_since_last as f64).powi(2));
        let freq_score = r.frequency_30d as f64 / max_freq;
        let weekday_score = (r.weekday_frequency as f64 / 8.0).min(1.0); // ~8 weeks of that weekday
        let streak_score = if r.days_since_last <= 1 { 1.0 } else if r.days_since_last <= 2 { 0.5 } else { 0.0 };

        let score = 0.35 * recency_score + 0.25 * weekday_score + 0.25 * freq_score + 0.15 * streak_score;

        let reason = if r.days_since_last == 0 {
            "Active today".to_string()
        } else if streak_score > 0.5 {
            format!("Worked on {} days recently", r.total_days)
        } else if weekday_score > 0.3 {
            format!("Common on {}", weekday_name(weekday))
        } else if freq_score > 0.5 {
            format!("Frequent ({} days/month)", r.frequency_30d)
        } else {
            String::new()
        };

        score_map.insert(r.task_id, (score, reason));
    }

    // Fetch Odoo tasks and merge with scores
    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let tasks = if let Some(client) = odoo_client {
        let uid = client.uid();
        match client.get_my_tasks(uid).await {
            Ok(tasks) => {
                let infos: Vec<TaskInfo> = tasks.into_iter().map(odoo_task_to_info).collect();
                if let Ok(db) = state.db.lock() {
                    let _ = cache_tasks(&db, &infos);
                }
                infos
            }
            Err(e) => {
                log::error!("get_suggested_tasks: Odoo error, using cache: {e}");
                let db = state.db.lock().unwrap();
                db_get_recent_tasks(&db, 30)?
                    .into_iter()
                    .map(|c| TaskInfo {
                        id: c.id,
                        name: c.name,
                        project_id: c.project_id,
                        project_name: c.project_name,
                        stage_id: None,
                        stage_name: c.stage_name,
                        state: None,
                        kanban_state: None,
                        planned_hours: c.planned_hours,
                        effective_hours: c.effective_hours,
                        description: None,
                        priority: None,
                        date_deadline: None,
                        write_date: None,
                        create_date: None,
                        user_ids: None,
                        parent_id: None,
                        color: None,
                    })
                    .collect()
            }
        }
    } else {
        let db = state.db.lock().unwrap();
        db_get_recent_tasks(&db, 30)?
            .into_iter()
            .map(|c| TaskInfo {
                id: c.id,
                name: c.name,
                project_id: c.project_id,
                project_name: c.project_name,
                stage_id: None,
                stage_name: c.stage_name,
                state: None,
                kanban_state: None,
                planned_hours: c.planned_hours,
                effective_hours: c.effective_hours,
                description: None,
                priority: None,
                date_deadline: None,
                write_date: None,
                create_date: None,
                user_ids: None,
                parent_id: None,
                color: None,
            })
            .collect()
    };

    // Also include tasks from ranking that might not be in "my tasks" but have strong patterns
    let my_task_ids: std::collections::HashSet<i64> = tasks.iter().map(|t| t.id).collect();
    let mut extra_from_patterns: Vec<SuggestedTask> = ranking_data
        .iter()
        .filter(|r| !my_task_ids.contains(&r.task_id))
        .filter_map(|r| {
            let (score, reason) = score_map.get(&r.task_id)?;
            if *score < 0.2 {
                return None;
            }
            Some(SuggestedTask {
                id: r.task_id,
                name: r.task_name.clone(),
                project_id: None,
                project_name: Some(r.project_name.clone()),
                stage_id: None,
                stage_name: None,
                state: None,
                kanban_state: None,
                planned_hours: 0.0,
                effective_hours: 0.0,
                description: None,
                priority: None,
                date_deadline: None,
                write_date: None,
                create_date: None,
                user_ids: None,
                parent_id: None,
                color: None,
                score: *score,
                reason: if reason.is_empty() { None } else { Some(reason.clone()) },
            })
        })
        .collect();

    // Build final list with scores
    let mut suggested: Vec<SuggestedTask> = tasks
        .into_iter()
        .map(|t| {
            let (score, reason) = score_map
                .get(&t.id)
                .cloned()
                .unwrap_or((0.05, String::new())); // low default score for unscored tasks
            SuggestedTask {
                id: t.id,
                name: t.name,
                project_id: t.project_id,
                project_name: t.project_name,
                stage_id: t.stage_id,
                stage_name: t.stage_name,
                state: t.state,
                kanban_state: t.kanban_state,
                planned_hours: t.planned_hours,
                effective_hours: t.effective_hours,
                description: t.description,
                priority: t.priority,
                date_deadline: t.date_deadline,
                write_date: t.write_date,
                create_date: t.create_date,
                user_ids: t.user_ids,
                parent_id: t.parent_id,
                color: t.color,
                score,
                reason: if reason.is_empty() { None } else { Some(reason) },
            }
        })
        .collect();

    suggested.append(&mut extra_from_patterns);
    suggested.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    suggested.dedup_by_key(|t| t.id);
    suggested.truncate(30);

    log::info!(
        "get_suggested_tasks: returning {} tasks (top score: {:.3})",
        suggested.len(),
        suggested.first().map(|t| t.score).unwrap_or(0.0)
    );

    Ok(suggested)
}

fn weekday_name(w: u32) -> &'static str {
    match w {
        0 => "Sundays",
        1 => "Mondays",
        2 => "Tuesdays",
        3 => "Wednesdays",
        4 => "Thursdays",
        5 => "Fridays",
        6 => "Saturdays",
        _ => "this day",
    }
}

// ── Task stage management ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct StageInfo {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskStageInfo {
    pub stage_id: Option<i64>,
    pub stage_name: Option<String>,
    pub state: Option<String>,
    pub kanban_state: Option<String>,
    pub available_stages: Vec<StageInfo>,
}

#[tauri::command]
pub async fn get_task_stages(
    task_id: i64,
    project_id: i64,
    state: tauri::State<'_, AppState>,
) -> AppResult<TaskStageInfo> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    // Fetch current state and available stages in parallel
    let (full_state, stages) = tokio::join!(
        client.get_task_full_state(task_id),
        client.get_project_stages(project_id),
    );

    let (stage, task_state, kanban) = full_state.unwrap_or((None, None, None));
    let stages = stages.unwrap_or_default();

    Ok(TaskStageInfo {
        stage_id: stage.as_ref().map(|(id, _)| *id),
        stage_name: stage.map(|(_, name)| name),
        state: task_state,
        kanban_state: kanban,
        available_stages: stages
            .into_iter()
            .map(|(id, name)| StageInfo { id, name })
            .collect(),
    })
}

#[tauri::command]
pub async fn update_task_stage(
    task_id: i64,
    stage_id: i64,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_stage(task_id, stage_id).await
}

#[tauri::command]
pub async fn update_task_kanban_state(
    task_id: i64,
    kanban_state: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_kanban_state(task_id, &kanban_state).await
}

#[tauri::command]
pub async fn update_task_state(
    task_id: i64,
    new_state: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_state(task_id, &new_state).await
}

// ── Task detail commands ────────────────────────────────────────────

#[tauri::command]
pub async fn update_task_name(
    task_id: i64,
    name: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_name(task_id, &name).await
}

#[tauri::command]
pub async fn update_task_description(
    task_id: i64,
    description: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_description(task_id, &description).await
}

#[tauri::command]
pub async fn update_task_deadline(
    task_id: i64,
    date_deadline: Option<String>,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_deadline(task_id, date_deadline.as_deref()).await
}

#[tauri::command]
pub async fn update_task_priority(
    task_id: i64,
    priority: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<bool> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.update_task_priority(task_id, &priority).await
}

#[tauri::command]
pub async fn get_task_details(
    task_id: i64,
    state: tauri::State<'_, AppState>,
) -> AppResult<TaskInfo> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    let task = client.get_task_details(task_id).await?;
    Ok(odoo_task_to_info(task))
}
