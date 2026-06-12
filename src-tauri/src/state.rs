use std::sync::Mutex;
use std::time::Instant;

use crate::odoo::attendance::AttendanceStatus;
use crate::odoo::client::OdooClient;
use crate::reminder::ReminderState;
use crate::timer::engine::TimerEngine;

/// In-memory cache entry with TTL
pub struct CacheEntry<T> {
    pub data: T,
    #[allow(dead_code)]
    pub fetched_at: Instant,
}

impl<T: Clone> CacheEntry<T> {
    pub fn new(data: T) -> Self {
        Self { data, fetched_at: Instant::now() }
    }

    #[allow(dead_code)]
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        self.fetched_at.elapsed().as_secs() > max_age_secs
    }
}

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub odoo: Mutex<Option<OdooClient>>,
    pub timer: Mutex<TimerEngine>,
    pub reminder: Mutex<ReminderState>,
    pub last_attendance: Mutex<Option<AttendanceStatus>>,
    /// In-memory cache for my_tasks (avoids repeated Odoo round-trips)
    pub tasks_cache: Mutex<Option<CacheEntry<Vec<crate::commands::tasks::TaskInfo>>>>,
    /// In-memory cache for projects
    #[allow(dead_code)]
    pub projects_cache: Mutex<Option<CacheEntry<Vec<crate::commands::projects::ProjectInfo>>>>,
    /// Prevents concurrent sync operations
    pub sync_in_progress: Mutex<bool>,
    /// Pending update handle from the updater plugin (desktop only)
    #[cfg(desktop)]
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
}
