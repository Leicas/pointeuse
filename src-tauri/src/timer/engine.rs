use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{AppError, AppResult};

// ── Public data types ────────────────────────────────────────────────

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
            },
            TimerState::Running {
                task_id,
                task_name,
                project_id,
                project_name,
                start_time,
                accumulated_secs,
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
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, TimerState::Running { .. })
    }

    /// Restore a previously persisted timer (crash recovery on startup).
    pub fn restore(
        &mut self,
        task_id: i64,
        task_name: String,
        project_id: i64,
        project_name: String,
        start_time: DateTime<Utc>,
        accumulated_secs: u64,
    ) {
        self.state = TimerState::Running {
            task_id,
            task_name,
            project_id,
            project_name,
            start_time,
            accumulated_secs,
        };
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
