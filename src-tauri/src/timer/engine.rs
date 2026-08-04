use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{AppError, AppResult};

// ── Public data types ────────────────────────────────────────────────

/// Cross-device identity of a timer run.
///
/// Every timer run gets a `session_id` that is stable across devices: the
/// device that started it stamps its own id/label as the origin, and any other
/// instance that adopts the run through Odoo reuses the same triple. Once the
/// run has been published to Odoo, `odoo_line_id` points at the
/// `account.analytic.line` that mirrors it.
#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub origin_device: String,
    pub origin_label: String,
    pub odoo_line_id: Option<i64>,
}

impl SessionMeta {
    /// A session that is not tracked across devices (sync disabled / offline start).
    pub fn local_only() -> Self {
        Self {
            session_id: String::new(),
            origin_device: String::new(),
            origin_label: String::new(),
            odoo_line_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum TimerState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "running")]
    Running {
        task_id: i64,
        task_name: String,
        project_id: i64,
        project_name: String,
        start_time: DateTime<Utc>,
        accumulated_secs: u64,
        session: SessionMeta,
    },
}

/// Returned to the frontend after a successful `stop`.
#[derive(Debug, Clone, Serialize)]
pub struct TimerResult {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub elapsed_secs: u64,
    pub session_id: String,
    pub odoo_line_id: Option<i64>,
}

/// Snapshot of the current timer state with a computed `elapsed_secs`.
#[derive(Debug, Clone, Serialize)]
pub struct TimerStateInfo {
    pub is_running: bool,
    pub task_id: Option<i64>,
    pub task_name: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub elapsed_secs: u64,
    /// Cross-device session id (empty when the run is not synced).
    pub session_id: String,
    /// Id of the device this run was started on (empty when not synced).
    pub origin_device: String,
    /// Human-readable label of that device, for "started on <X>" hints.
    pub origin_label: String,
}

// ── Engine ───────────────────────────────────────────────────────────

pub struct TimerEngine {
    state: TimerState,
}

impl TimerEngine {
    pub fn new() -> Self {
        Self {
            state: TimerState::Idle,
        }
    }

    /// Start tracking time for a task.  Errors if a timer is already running.
    pub fn start(
        &mut self,
        task_id: i64,
        task_name: String,
        project_id: i64,
        project_name: String,
        session: SessionMeta,
    ) -> AppResult<()> {
        if self.is_running() {
            return Err(AppError::Timer(
                "Timer is already running".to_string(),
            ));
        }
        self.state = TimerState::Running {
            task_id,
            task_name,
            project_id,
            project_name,
            start_time: Utc::now(),
            accumulated_secs: 0,
            session,
        };
        Ok(())
    }

    /// Stop the timer and return the result.  Errors if idle.
    pub fn stop(&mut self) -> AppResult<TimerResult> {
        match std::mem::replace(&mut self.state, TimerState::Idle) {
            TimerState::Running {
                task_id,
                task_name,
                project_id,
                project_name,
                start_time,
                accumulated_secs,
                session,
            } => {
                let wall_secs = Utc::now()
                    .signed_duration_since(start_time)
                    .num_seconds()
                    .max(0) as u64;
                let elapsed_secs = accumulated_secs + wall_secs;
                Ok(TimerResult {
                    task_id,
                    task_name,
                    project_id,
                    project_name,
                    elapsed_secs,
                    session_id: session.session_id,
                    odoo_line_id: session.odoo_line_id,
                })
            }
            TimerState::Idle => Err(AppError::Timer(
                "No timer is running".to_string(),
            )),
        }
    }

    /// Discard the running timer without producing a result.
    pub fn discard(&mut self) -> AppResult<()> {
        if !self.is_running() {
            return Err(AppError::Timer(
                "No timer is running".to_string(),
            ));
        }
        self.state = TimerState::Idle;
        Ok(())
    }

    /// Return a snapshot of the current state with a computed `elapsed_secs`.
    pub fn get_state(&self) -> TimerStateInfo {
        match &self.state {
            TimerState::Idle => TimerStateInfo {
                is_running: false,
                task_id: None,
                task_name: None,
                project_id: None,
                project_name: None,
                elapsed_secs: 0,
                session_id: String::new(),
                origin_device: String::new(),
                origin_label: String::new(),
            },
            TimerState::Running {
                task_id,
                task_name,
                project_id,
                project_name,
                start_time,
                accumulated_secs,
                session,
            } => {
                let wall_secs = Utc::now()
                    .signed_duration_since(*start_time)
                    .num_seconds()
                    .max(0) as u64;
                TimerStateInfo {
                    is_running: true,
                    task_id: Some(*task_id),
                    task_name: Some(task_name.clone()),
                    project_id: Some(*project_id),
                    project_name: Some(project_name.clone()),
                    elapsed_secs: accumulated_secs + wall_secs,
                    session_id: session.session_id.clone(),
                    origin_device: session.origin_device.clone(),
                    origin_label: session.origin_label.clone(),
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, TimerState::Running { .. })
    }

    /// Restore a previously persisted timer (crash recovery on startup).
    ///
    /// Also used to adopt a run that another device started: unlike `start`,
    /// this overwrites whatever the engine currently holds, because the caller
    /// has already decided which session wins.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        &mut self,
        task_id: i64,
        task_name: String,
        project_id: i64,
        project_name: String,
        start_time: DateTime<Utc>,
        accumulated_secs: u64,
        session: SessionMeta,
    ) {
        self.state = TimerState::Running {
            task_id,
            task_name,
            project_id,
            project_name,
            start_time,
            accumulated_secs,
            session,
        };
    }

    /// Attach the Odoo line that now mirrors the running session.
    /// No-op when the timer moved on to a different session in the meantime.
    pub fn attach_odoo_line(&mut self, session_id: &str, line_id: i64) -> bool {
        match &mut self.state {
            TimerState::Running { session, .. } if session.session_id == session_id => {
                session.odoo_line_id = Some(line_id);
                true
            }
            _ => false,
        }
    }

    /// Session metadata of the running timer, if any.
    pub fn session(&self) -> Option<&SessionMeta> {
        match &self.state {
            TimerState::Running { session, .. } => Some(session),
            TimerState::Idle => None,
        }
    }

    /// Borrow the raw state (used by persistence layer).
    pub fn raw_state(&self) -> &TimerState {
        &self.state
    }
}

impl Default for TimerEngine {
    fn default() -> Self {
        Self::new()
    }
}
