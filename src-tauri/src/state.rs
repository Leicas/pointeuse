use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::devicesync::{DeviceIdentity, PendingLog};
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
    /// Stable identity of this install, stamped on live sessions it starts.
    /// Mutable so a rename in settings takes effect without a restart.
    pub device: Mutex<DeviceIdentity>,
    /// Serialises everything that touches a live session's Odoo line, so the
    /// background reconciler and a user-driven `log_time` cannot both write it
    pub sync_lock: Arc<tokio::sync::Mutex<()>>,
    /// Lets commands wake the reconciler instead of waiting out its interval
    pub sync_wakeup: Arc<tokio::sync::Notify>,
    /// The session whose log form is currently open, if any
    pub pending_log: Mutex<Option<PendingLog>>,
    /// Pending update handle from the updater plugin (desktop only)
    #[cfg(desktop)]
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
}
