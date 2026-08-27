//! Cross-device timer sync.
//!
//! Every instance logged into the same Odoo server with the same user should
//! agree on what is currently being tracked: start a timer on the phone and the
//! desktop picks it up, stop it on the desktop and the phone follows.
//!
//! There is no Pointeuse server, so Odoo is the only shared medium. A running
//! timer is mirrored as an `account.analytic.line` whose description carries a
//! marker (see [`MARKER`]); that is a record type the app already creates as
//! part of its normal job, so it needs no extra rights and no custom module.
//!
//! **Odoo is never on the critical path.** The local [`TimerEngine`] stays
//! authoritative and every command returns without touching the network; a
//! background reconciler pushes local changes up, pulls remote ones down, and
//! drains an outbox of writes that must survive being offline or killed.
//!
//! Conflict rule: if two devices somehow run a timer at once, the one that
//! started most recently wins and the loser's elapsed time is written to Odoo
//! rather than discarded.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

use crate::db::session::{self as outbox, OutboxAction, OutboxEntry};
use crate::db::tasks::touch_recent_at;
use crate::odoo::client::OdooClient;
use crate::state::AppState;
use crate::timer::engine::{SessionMeta, TimerResult};
use crate::timer::persistence::{clear_timer_state, save_timer_state};

/// Sentinel that turns a timesheet line into a live session. ASCII on purpose:
/// it travels through XML-RPC `like` domains and Odoo list views unharmed.
pub const MARKER: &str = "#PTZ1#";

/// How often the owning device refreshes the elapsed hours on its live line.
const HEARTBEAT_SECS: u64 = 60;
// Poll cadence. The responsive path is really `nudge()` — window focus and the
// timer commands trigger an immediate pass — so the periodic poll is a safety
// net rather than the mechanism. It therefore starts brisk and backs off while
// nothing is happening, instead of hammering a slow Odoo all day for nothing.
/// First interval after any activity, window on screen.
const POLL_FOREGROUND_SECS: u64 = 15;
/// First interval after any activity, minimised to the tray or backgrounded.
const POLL_BACKGROUND_SECS: u64 = 60;
/// Ceilings once the reconciler has had nothing to do for a while.
const POLL_FOREGROUND_MAX_SECS: u64 = 120;
const POLL_BACKGROUND_MAX_SECS: u64 = 600;
/// Extra clamp on the poll while Odoo cannot be reached at all, applied on
/// mobile only. Android's background network policy can cut the app off
/// entirely (every request dies with a DNS error), and the webview still
/// reports itself visible, so the quiet backoff alone keeps retrying at the
/// brisk foreground cadence. Double per consecutive failure up to 10 min; a
/// success or a `nudge()` (app resume, timer command) resets it. Desktop keeps
/// its normal cadence.
const NET_FAILURE_BASE_SECS: u64 = 60;
const NET_FAILURE_MAX_SECS: u64 = 600;
/// The retries are identical while the network is down, so log the first
/// consecutive failure and then only every Nth.
const FAILURE_LOG_EVERY: u32 = 10;
/// Consecutive empty polls before concluding another device ended the run.
/// One is not enough: a single failed or racy read would drop a live timer.
const MISSING_POLLS_BEFORE_STOP: u32 = 2;
/// How often shared recents are rebuilt from Odoo timesheet history.
const RECENTS_REFRESH_SECS: u64 = 600;
/// How far back that history reaches.
const RECENTS_WINDOW_DAYS: i64 = 30;

// ── Marker codec ─────────────────────────────────────────────────────

/// The live-session header parsed out of a timesheet line's description.
#[derive(Debug, Clone)]
pub struct SessionMarker {
    pub session_id: String,
    pub device_id: String,
    pub device_label: String,
    pub start: DateTime<Utc>,
}

/// `#` separates marker fields, so it cannot appear inside one.
fn sanitize(field: &str) -> String {
    field
        .chars()
        .map(|c| if c == '#' || c.is_control() { '-' } else { c })
        .collect()
}

/// Append the live-session marker to a description.
pub fn mark_description(description: &str, session: &SessionMeta, start: DateTime<Utc>) -> String {
    format!(
        "{} {MARKER}{}#{}#{}#{}#",
        description.trim(),
        sanitize(&session.session_id),
        sanitize(&session.origin_device),
        sanitize(&session.origin_label),
        start.timestamp(),
    )
}

/// Split a description into its user-visible part and its marker, if any.
pub fn split_marker(name: &str) -> (String, Option<SessionMarker>) {
    let Some(idx) = name.find(MARKER) else {
        return (name.to_string(), None);
    };
    let clean = name[..idx].trim_end().to_string();
    let parts: Vec<&str> = name[idx + MARKER.len()..].split('#').collect();
    if parts.len() < 4 {
        return (clean, None);
    }
    let Some(start) = parts[3]
        .parse::<i64>()
        .ok()
        .and_then(|secs| Utc.timestamp_opt(secs, 0).single())
    else {
        return (clean, None);
    };
    if parts[0].is_empty() {
        return (clean, None);
    }
    (
        clean,
        Some(SessionMarker {
            session_id: parts[0].to_string(),
            device_id: parts[1].to_string(),
            device_label: parts[2].to_string(),
            start,
        }),
    )
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    fn session(id: &str, device: &str, label: &str) -> SessionMeta {
        SessionMeta {
            session_id: id.into(),
            origin_device: device.into(),
            origin_label: label.into(),
            odoo_line_id: None,
        }
    }

    #[test]
    fn round_trips() {
        let start = Utc.timestamp_opt(1_754_300_000, 0).unwrap();
        let marked = mark_description("Fix the login bug", &session("s1", "dev1", "Pixel 8"), start);
        let (clean, parsed) = split_marker(&marked);
        let parsed = parsed.expect("marker should parse");
        assert_eq!(clean, "Fix the login bug");
        assert_eq!(parsed.session_id, "s1");
        assert_eq!(parsed.device_id, "dev1");
        assert_eq!(parsed.device_label, "Pixel 8");
        assert_eq!(parsed.start, start);
    }

    #[test]
    fn leaves_ordinary_descriptions_alone() {
        let (clean, parsed) = split_marker("Reviewed PR #123 (#4 in the queue)");
        assert_eq!(clean, "Reviewed PR #123 (#4 in the queue)");
        assert!(parsed.is_none());
    }

    #[test]
    fn a_hash_in_the_description_survives_the_round_trip() {
        let start = Utc.timestamp_opt(1_754_300_000, 0).unwrap();
        let marked = mark_description("Fix #123", &session("s2", "dev2", "Desk#top"), start);
        let (clean, parsed) = split_marker(&marked);
        assert_eq!(clean, "Fix #123");
        // The label's `#` is the field separator, so it is replaced on the way in.
        assert_eq!(parsed.unwrap().device_label, "Desk-top");
    }

    #[test]
    fn the_poll_backs_off_while_nothing_happens() {
        // Brisk right after activity, then doubling to a ceiling.
        assert_eq!(poll_interval(true, 0), 15);
        assert_eq!(poll_interval(true, 1), 30);
        assert_eq!(poll_interval(true, 3), 120);
        assert_eq!(poll_interval(true, 99), 120, "foreground must stay capped");

        assert_eq!(poll_interval(false, 0), 60);
        assert_eq!(poll_interval(false, 99), 600, "background must stay capped");

        // Never faster than the base, however the counter is fed.
        assert!(poll_interval(true, 0) <= poll_interval(true, 1));
    }

    #[test]
    fn network_failures_back_off_and_log_sparsely() {
        // Doubling per consecutive failure, capped at ten minutes.
        assert_eq!(net_failure_backoff(1), 60);
        assert_eq!(net_failure_backoff(2), 120);
        assert_eq!(net_failure_backoff(4), 480);
        assert_eq!(net_failure_backoff(5), 600);
        assert_eq!(net_failure_backoff(99), 600, "must stay capped");

        // First failure logged, then only every Nth.
        assert!(should_log_failure(1));
        assert!(!should_log_failure(2));
        assert!(should_log_failure(FAILURE_LOG_EVERY));
        assert!(!should_log_failure(FAILURE_LOG_EVERY + 1));
        assert!(should_log_failure(FAILURE_LOG_EVERY * 2));
    }

    #[test]
    fn rejects_a_truncated_marker() {
        let (clean, parsed) = split_marker("Some work #PTZ1#s3#dev3#");
        assert_eq!(clean, "Some work");
        assert!(parsed.is_none());
    }
}

// ── Device identity ──────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceIdentity {
    pub id: String,
    pub label: String,
}

/// 16 hex chars from a fresh `RandomState` seed mixed with the wall clock —
/// enough to keep device and session ids apart without pulling in a uuid crate.
fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let seed = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}", seed ^ nanos.rotate_left(17))
}

fn default_label() -> String {
    #[cfg(target_os = "android")]
    {
        "Android".to_string()
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Desktop".to_string())
    }
}

/// Read this install's identity from the settings store, minting one on first run.
pub fn load_identity(app: &AppHandle) -> DeviceIdentity {
    let Ok(store) = app.store("settings.json") else {
        return DeviceIdentity {
            id: random_id(),
            label: default_label(),
        };
    };

    let id = store
        .get("device_id")
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let fresh = random_id();
            store.set("device_id", serde_json::Value::String(fresh.clone()));
            let _ = store.save();
            fresh
        });

    let label = store
        .get("device_label")
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_label);

    DeviceIdentity { id, label }
}

/// Whether cross-device sync is switched on (default: yes).
pub fn sync_enabled(app: &AppHandle) -> bool {
    app.store("settings.json")
        .ok()
        .and_then(|s| s.get("device_sync_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

// ── Hooks called from the timer commands ─────────────────────────────

/// Mint the session identity for a run started on this device.
/// Returns a local-only identity when sync is off, which disables every code
/// path below for that run.
pub fn new_session(app: &AppHandle) -> SessionMeta {
    if !sync_enabled(app) {
        return SessionMeta::local_only();
    }
    let state = app.state::<AppState>();
    let device = state.device.lock().unwrap();
    SessionMeta {
        session_id: random_id(),
        origin_device: device.id.clone(),
        origin_label: device.label.clone(),
        odoo_line_id: None,
    }
}

/// Ask the reconciler to run now instead of waiting out its poll interval.
pub fn nudge(app: &AppHandle) {
    app.state::<AppState>().sync_wakeup.notify_one();
}

/// Hand a finished run to the outbox, which owns its Odoo side from here on.
///
/// Returns `true` when the outbox took ownership; the caller must then *not*
/// create a timesheet line itself, or the run would be logged twice.
pub fn enqueue_finish(
    conn: &Connection,
    result: &TimerResult,
    action: OutboxAction,
    description: &str,
    date: &str,
) -> bool {
    if result.session_id.is_empty() {
        return false; // not a synced run — classic local path applies
    }
    let entry = OutboxEntry {
        session_id: result.session_id.clone(),
        odoo_line_id: result.odoo_line_id,
        action,
        task_id: result.task_id,
        project_id: result.project_id,
        task_name: result.task_name.clone(),
        project_name: result.project_name.clone(),
        description: description.to_string(),
        hours: result.elapsed_secs as f64 / 3600.0,
        date: date.to_string(),
    };
    match outbox::enqueue(conn, &entry) {
        Ok(()) => true,
        Err(e) => {
            log::error!("devicesync: could not queue session {}: {e}", result.session_id);
            false
        }
    }
}

/// Which session the log form that is currently open belongs to.
#[derive(Debug, Clone)]
pub struct PendingLog {
    pub session_id: String,
    pub task_id: i64,
    pub odoo_line_id: Option<i64>,
    /// Guards against a form that was never submitted attaching itself to some
    /// unrelated `log_time` call hours later.
    pub stopped_at: DateTime<Utc>,
}

/// How long a stopped run stays amendable by its log form.
pub const PENDING_LOG_TTL_MINS: i64 = 60;

// ── Outbox flushing ──────────────────────────────────────────────────

/// Outcome of trying to settle one outbox row.
pub(crate) enum Flushed {
    /// Settled — drop the row. Carries the line it landed on, when known; a
    /// `None` line means the work went to the offline pending-timesheet queue,
    /// which is durable in its own right.
    Done(Option<i64>),
    /// Odoo refused; keep the row and try again on the next pass.
    Retry,
}

/// Settle one finished session in Odoo.
pub(crate) async fn flush_one(
    app: &AppHandle,
    client: &OdooClient,
    entry: &OutboxEntry,
) -> Flushed {
    match flush_inner(app, client, entry).await {
        Flushed::Done(line) => Flushed::Done(line.or(entry.odoo_line_id)),
        Flushed::Retry => Flushed::Retry,
    }
}

async fn flush_inner(app: &AppHandle, client: &OdooClient, entry: &OutboxEntry) -> Flushed {
    match (entry.action, entry.odoo_line_id) {
        // Published run: strip the marker and write the final numbers.
        (OutboxAction::Finalize, Some(line_id)) => {
            let description = if entry.description.trim().is_empty() {
                entry.task_name.clone()
            } else {
                entry.description.clone()
            };
            match client
                .finalize_live_line(
                    line_id,
                    entry.task_id,
                    entry.project_id,
                    &description,
                    entry.hours,
                    &entry.date,
                )
                .await
            {
                Ok(_) => {
                    // Keep the local history in step with what Odoo now holds.
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = crate::db::timesheets::delete_log_by_odoo_id(&db, line_id);
                    let _ = crate::db::timesheets::add_to_log(
                        &db,
                        entry.task_id,
                        &entry.task_name,
                        &entry.project_name,
                        &description,
                        entry.hours,
                        &entry.date,
                        Some(line_id),
                    );
                    Flushed::Done(Some(line_id))
                }
                Err(e) => {
                    log::warn!("devicesync: finalize of line {line_id} failed, will retry: {e}");
                    Flushed::Retry
                }
            }
        }
        // Never reached Odoo: fall back to the normal logging path, which
        // handles the private-task redirect and queues when offline. It is
        // durable either way, so the row is done regardless of the outcome.
        (OutboxAction::Finalize, None) => {
            let created = crate::commands::timesheet::log_time_with_fallback(
                app,
                client,
                entry.task_id,
                entry.project_id,
                &entry.task_name,
                &entry.project_name,
                entry.hours,
                &entry.date,
            )
            .await;
            Flushed::Done(created)
        }
        (OutboxAction::Discard, Some(line_id)) => {
            match client.unlink("account.analytic.line", vec![line_id]).await {
                Ok(_) => Flushed::Done(None),
                Err(e) => {
                    log::warn!("devicesync: discard of line {line_id} failed, will retry: {e}");
                    Flushed::Retry
                }
            }
        }
        (OutboxAction::Discard, None) => Flushed::Done(None),
    }
}

/// Tell an open log form where its session's line ended up, so a later submit
/// amends that line instead of creating a duplicate.
pub(crate) fn note_pending_line(app: &AppHandle, session_id: &str, line_id: Option<i64>) {
    let Some(line_id) = line_id else { return };
    let state = app.state::<AppState>();
    let mut pending = state.pending_log.lock().unwrap();
    if let Some(p) = pending.as_mut() {
        if p.session_id == session_id {
            p.odoo_line_id = Some(line_id);
        }
    }
}

/// Drain the outbox. Rows survive until Odoo accepts them.
/// Returns whether anything was settled, which keeps the poll cadence brisk.
async fn flush_outbox(app: &AppHandle, client: &OdooClient) -> bool {
    let entries = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        outbox::list(&db).unwrap_or_default()
    };
    if entries.is_empty() {
        return false;
    }

    let mut settled = 0usize;
    for entry in entries {
        match flush_inner(app, client, &entry).await {
            Flushed::Retry => continue,
            Flushed::Done(line) => {
                settled += 1;
                {
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = outbox::remove(&db, &entry.session_id);
                }
                note_pending_line(app, &entry.session_id, line.or(entry.odoo_line_id));
            }
        }
    }

    if settled > 0 {
        let _ = app.emit("timesheet_changed", ());
    }
    settled > 0
}

// ── Push: local run → Odoo ───────────────────────────────────────────

/// Snapshot of the running timer, taken without holding a lock across awaits.
#[derive(Debug, Clone)]
struct LocalRun {
    session_id: String,
    origin_device: String,
    origin_label: String,
    odoo_line_id: Option<i64>,
    task_id: i64,
    project_id: i64,
    task_name: String,
    start: DateTime<Utc>,
    elapsed_secs: u64,
}

fn local_run(app: &AppHandle) -> Option<LocalRun> {
    let state = app.state::<AppState>();
    let timer = state.timer.lock().unwrap();
    let session = timer.session()?.clone();
    if session.session_id.is_empty() {
        return None; // local-only run, nothing to sync
    }
    let info = timer.get_state();
    let start = Utc::now() - chrono::Duration::seconds(info.elapsed_secs as i64);
    Some(LocalRun {
        session_id: session.session_id,
        origin_device: session.origin_device,
        origin_label: session.origin_label,
        odoo_line_id: session.odoo_line_id,
        task_id: info.task_id.unwrap_or(0),
        project_id: info.project_id.unwrap_or(0),
        task_name: info.task_name.unwrap_or_default(),
        start,
        elapsed_secs: info.elapsed_secs,
    })
}

/// Publish a newly started run, or refresh the hours on an already published one.
///
/// `line_is_live` is what the pull earlier in this pass established about our
/// own session: `Some(true)` it is still marked live in Odoo, `Some(false)` it
/// is not, `None` Odoo could not be reached. Heartbeats only go out on
/// `Some(true)` — writing hours onto a line another device already finalized
/// would inflate the entry, and after a long sleep it would inflate it badly.
///
/// Returns whether it published something. A heartbeat does not count: it
/// recurs for as long as the timer runs, and treating it as activity would pin
/// the poll at its fastest cadence for the whole session.
async fn push(
    app: &AppHandle,
    client: &OdooClient,
    my_device: &str,
    line_is_live: Option<bool>,
    last_heartbeat: &mut Option<(String, Instant)>,
) -> bool {
    let Some(run) = local_run(app) else { return false };

    // Only the device that started the run writes to its line. A device that
    // adopted the run just mirrors it and computes elapsed from the start time.
    if run.origin_device != my_device {
        return false;
    }

    match run.odoo_line_id {
        None => {
            let session = SessionMeta {
                session_id: run.session_id.clone(),
                origin_device: run.origin_device.clone(),
                origin_label: run.origin_label.clone(),
                odoo_line_id: None,
            };
            let marked = mark_description(&run.task_name, &session, run.start);
            let date = run.start.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();

            let line_id = match client
                .create_live_line(run.task_id, run.project_id, &marked, &date)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    // Offline or a rejected task: the timer keeps running
                    // locally and we retry on the next pass.
                    log::warn!("devicesync: could not publish session {}: {e}", run.session_id);
                    return false;
                }
            };

            // The run may have ended while the create was in flight.
            let orphaned = {
                let state = app.state::<AppState>();
                let db = state.db.lock().unwrap();
                let mut timer = state.timer.lock().unwrap();
                if timer.attach_odoo_line(&run.session_id, line_id) {
                    let _ = save_timer_state(&db, &timer);
                    false
                } else {
                    // Stopped meanwhile — hand the line to the queued finish.
                    !outbox::attach_line(&db, &run.session_id, line_id).unwrap_or(false)
                }
            };

            if orphaned {
                log::info!("devicesync: session {} vanished, removing line {line_id}", run.session_id);
                let _ = client.unlink("account.analytic.line", vec![line_id]).await;
            } else {
                *last_heartbeat = Some((run.session_id.clone(), Instant::now()));
            }
            true
        }
        Some(line_id) => {
            if line_is_live != Some(true) {
                return false;
            }
            let due = match last_heartbeat {
                Some((sid, at)) if *sid == run.session_id => {
                    at.elapsed() >= Duration::from_secs(HEARTBEAT_SECS)
                }
                _ => true,
            };
            if !due {
                return false;
            }
            let hours = run.elapsed_secs as f64 / 3600.0;
            match client.heartbeat_live_line(line_id, hours).await {
                Ok(_) => *last_heartbeat = Some((run.session_id.clone(), Instant::now())),
                Err(e) => log::warn!("devicesync: heartbeat for line {line_id} failed: {e}"),
            }
            false
        }
    }
}

// ── Pull: Odoo → local run ───────────────────────────────────────────

struct RemoteRun {
    line_id: i64,
    task_id: i64,
    task_name: String,
    project_id: i64,
    project_name: String,
    description: String,
    date: String,
    marker: SessionMarker,
}

/// Queue a finalize for a run that lost the newest-start contest.
fn finalize_loser(app: &AppHandle, remote: &RemoteRun) {
    let elapsed = Utc::now()
        .signed_duration_since(remote.marker.start)
        .num_seconds()
        .max(0) as u64;
    let entry = OutboxEntry {
        session_id: remote.marker.session_id.clone(),
        odoo_line_id: Some(remote.line_id),
        action: OutboxAction::Finalize,
        task_id: remote.task_id,
        project_id: remote.project_id,
        task_name: remote.task_name.clone(),
        project_name: remote.project_name.clone(),
        description: if remote.description.is_empty() {
            remote.task_name.clone()
        } else {
            remote.description.clone()
        },
        hours: elapsed as f64 / 3600.0,
        date: remote.date.clone(),
    };
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    if outbox::enqueue(&db, &entry).is_ok() {
        log::info!(
            "devicesync: session {} lost to a newer run, logging {:.2}h",
            remote.marker.session_id,
            entry.hours
        );
    }
}

/// Stop the local timer because another device ended the run. Nothing is
/// logged here: whichever device stopped it owns that write.
fn accept_remote_stop(app: &AppHandle, task_name: &str) {
    {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();
        let _ = timer.discard();
        let _ = clear_timer_state(&db);
        let mut reminder = state.reminder.lock().unwrap();
        reminder.reset_elapsed = true;
    }

    #[cfg(mobile)]
    {
        crate::notification::remove_ongoing_notification(app);
        crate::reminder::cancel_scheduled_reminder(app);
    }

    log::info!("devicesync: '{task_name}' was stopped on another device");
    let _ = app.emit(
        "timer_remote_stopped",
        serde_json::json!({ "task_name": task_name }),
    );
}

/// Take over a run started elsewhere.
fn adopt(app: &AppHandle, remote: &RemoteRun) {
    let elapsed = Utc::now()
        .signed_duration_since(remote.marker.start)
        .num_seconds()
        .max(0) as u64;
    let session = SessionMeta {
        session_id: remote.marker.session_id.clone(),
        origin_device: remote.marker.device_id.clone(),
        origin_label: remote.marker.device_label.clone(),
        odoo_line_id: Some(remote.line_id),
    };

    {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();
        timer.restore(
            remote.task_id,
            remote.task_name.clone(),
            remote.project_id,
            remote.project_name.clone(),
            remote.marker.start,
            0,
            session,
        );
        let _ = save_timer_state(&db, &timer);
        let _ = crate::db::tasks::touch_recent(
            &db,
            remote.task_id,
            &remote.task_name,
            Some(remote.project_name.as_str()),
        );
        let mut reminder = state.reminder.lock().unwrap();
        reminder.reset_elapsed = true;
    }

    #[cfg(mobile)]
    crate::notification::show_ongoing_notification(
        app,
        &remote.task_name,
        &remote.project_name,
        elapsed,
    );

    log::info!(
        "devicesync: adopted '{}' started on {} ({elapsed}s ago)",
        remote.task_name,
        remote.marker.device_label
    );
    let _ = app.emit(
        "timer_remote_started",
        serde_json::json!({
            "task_id": remote.task_id,
            "task_name": remote.task_name,
            "project_id": remote.project_id,
            "project_name": remote.project_name,
            "origin_label": remote.marker.device_label,
            "elapsed_secs": elapsed,
        }),
    );
}

/// What one pull established.
struct Pulled {
    /// Whether our own published line was seen live, or `None` when Odoo could
    /// not be reached — see [`push`] for why that distinction matters.
    ours_is_live: Option<bool>,
    /// Whether this device's timer state actually moved, which keeps the poll
    /// cadence brisk instead of letting it back off.
    changed: bool,
}

/// Reconcile what Odoo says is running with what this device is running.
async fn pull(
    app: &AppHandle,
    client: &OdooClient,
    missing_polls: &mut u32,
    net_failures: &mut u32,
) -> Pulled {
    let since = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();

    let lines = match client.find_live_lines(MARKER, &since).await {
        Ok(l) => {
            *net_failures = 0;
            l
        }
        Err(e) => {
            *net_failures = net_failures.saturating_add(1);
            if should_log_failure(*net_failures) {
                log::warn!(
                    "devicesync: live-session poll failed ({}x in a row): {e}",
                    *net_failures
                );
            }
            return Pulled { ours_is_live: None, changed: false };
        }
    };

    // Sessions already queued for settlement are zombies — their marker is
    // still in Odoo only because the finalize has not landed yet.
    let queued: Vec<String> = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        outbox::list(&db)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.session_id)
            .collect()
    };

    let mut remotes: Vec<RemoteRun> = Vec::new();
    for line in lines {
        let (description, marker) = split_marker(&line.name);
        let Some(marker) = marker else { continue };
        if queued.contains(&marker.session_id) {
            continue;
        }
        let (task_id, task_name) = line.task_id.unwrap_or((0, String::new()));
        let (project_id, project_name) = line.project_id.unwrap_or((0, String::new()));
        if task_id == 0 {
            continue; // a timesheet line with no task cannot drive a timer
        }
        remotes.push(RemoteRun {
            line_id: line.id,
            task_id,
            task_name,
            project_id,
            project_name,
            description,
            date: line.date,
            marker,
        });
    }

    let run = local_run(app);

    if remotes.is_empty() {
        // A published run that no longer exists in Odoo was ended elsewhere.
        let published = run
            .as_ref()
            .map(|r| r.odoo_line_id.is_some())
            .unwrap_or(false);
        let mut changed = false;
        if published {
            *missing_polls += 1;
            if *missing_polls >= MISSING_POLLS_BEFORE_STOP {
                *missing_polls = 0;
                accept_remote_stop(app, &run.map(|r| r.task_name).unwrap_or_default());
                changed = true;
            }
        } else {
            *missing_polls = 0;
        }
        return Pulled { ours_is_live: Some(false), changed };
    }
    *missing_polls = 0;

    // Is our own published line among what Odoo reports as live?
    let ours_is_live = run.as_ref().is_some_and(|r| {
        r.odoo_line_id.is_some()
            && remotes
                .iter()
                .any(|remote| remote.marker.session_id == r.session_id)
    });

    // Newest start wins, local run included.
    let newest_remote = remotes
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| r.marker.start)
        .map(|(i, _)| i)
        .unwrap();

    let local_wins = run
        .as_ref()
        .map(|r| r.start > remotes[newest_remote].marker.start)
        .unwrap_or(false);
    let winner_session = if local_wins {
        run.as_ref().map(|r| r.session_id.clone()).unwrap_or_default()
    } else {
        remotes[newest_remote].marker.session_id.clone()
    };

    let mut changed = false;
    for remote in &remotes {
        if remote.marker.session_id != winner_session {
            finalize_loser(app, remote);
            changed = true;
        }
    }

    if local_wins {
        return Pulled { ours_is_live: Some(ours_is_live), changed };
    }

    let winner = &remotes[newest_remote];
    match run {
        // Already tracking this run — just make sure we know its line.
        Some(ref r) if r.session_id == winner.marker.session_id => {
            if r.odoo_line_id.is_none() {
                let state = app.state::<AppState>();
                let db = state.db.lock().unwrap();
                let mut timer = state.timer.lock().unwrap();
                if timer.attach_odoo_line(&r.session_id, winner.line_id) {
                    let _ = save_timer_state(&db, &timer);
                    changed = true;
                }
            }
            return Pulled { ours_is_live: Some(true), changed };
        }
        // Our own run lost: hand it to the outbox, then take on the winner.
        Some(ref r) => {
            // Unless it has already been settled for us. A line we own that is
            // no longer marked live was finalized by whichever device started
            // the winning run — writing it again would only inflate the hours.
            let needs_settling = ours_is_live || r.odoo_line_id.is_none();
            let elapsed_hours = r.elapsed_secs as f64 / 3600.0;
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            if needs_settling {
                let state = app.state::<AppState>();
                let db = state.db.lock().unwrap();
                let entry = OutboxEntry {
                    session_id: r.session_id.clone(),
                    odoo_line_id: r.odoo_line_id,
                    action: OutboxAction::Finalize,
                    task_id: r.task_id,
                    project_id: r.project_id,
                    task_name: r.task_name.clone(),
                    project_name: String::new(),
                    description: r.task_name.clone(),
                    hours: elapsed_hours,
                    date,
                };
                let _ = outbox::enqueue(&db, &entry);
            }
            adopt(app, winner);
        }
        None => adopt(app, winner),
    }

    // We now mirror someone else's run; our own line, if any, was handed to the
    // outbox, so nothing here should be heartbeating.
    Pulled { ours_is_live: Some(false), changed: true }
}

// ── Shared recents ───────────────────────────────────────────────────

/// Seed the local recents list from this user's recent Odoo timesheet history,
/// so a task worked on from the phone shows up in the desktop's recents.
/// Derived rather than replicated: Odoo already knows what was worked on.
async fn pull_recent_tasks(app: &AppHandle, client: &OdooClient) {
    let today = chrono::Local::now();
    let from = (today - chrono::Duration::days(RECENTS_WINDOW_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    let to = today.format("%Y-%m-%d").to_string();

    let entries = match client.get_timesheets_for_range(&from, &to).await {
        Ok(e) => e,
        Err(e) => {
            log::warn!("devicesync: recents refresh failed: {e}");
            return;
        }
    };

    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let mut seeded = 0usize;
    for entry in entries {
        let Some((task_id, task_name)) = entry.task_id else { continue };
        let project_name = entry.project_id.map(|(_, n)| n).unwrap_or_default();
        // Midday keeps a date-only stamp from outranking a real local timestamp
        // recorded earlier the same day.
        let stamp = format!("{} 12:00:00", entry.date);
        if touch_recent_at(&db, task_id, &task_name, Some(&project_name), &stamp).is_ok() {
            seeded += 1;
        }
    }
    log::debug!("devicesync: merged {seeded} timesheet rows into recents");
}

// ── Auto-stop helper shared by the check-out paths ───────────────────

/// Stop a running timer on behalf of the app (external check-out, tray, idle
/// handling) and settle it in Odoo. Replaces the stop-then-log dance so that a
/// published live line is finalized instead of duplicated.
pub async fn auto_stop_timer(app: &AppHandle, client: &OdooClient) -> Option<TimerResult> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let lock = app.state::<AppState>().sync_lock.clone();
    let _guard = lock.lock().await;

    let (result, queued) = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let mut timer = state.timer.lock().unwrap();
        if !timer.is_running() {
            return None;
        }
        let result = match timer.stop() {
            Ok(r) => r,
            Err(e) => {
                log::error!("devicesync: auto-stop failed: {e}");
                return None;
            }
        };
        let _ = clear_timer_state(&db);
        let queued = enqueue_finish(&db, &result, OutboxAction::Finalize, &result.task_name, &date);
        (result, queued)
    };

    let hours = result.elapsed_secs as f64 / 3600.0;
    if queued {
        flush_outbox(app, client).await;
    } else {
        crate::commands::timesheet::log_time_with_fallback(
            app,
            client,
            result.task_id,
            result.project_id,
            &result.task_name,
            &result.project_name,
            hours,
            &date,
        )
        .await;
    }

    #[cfg(mobile)]
    {
        crate::notification::remove_ongoing_notification(app);
        crate::reminder::cancel_scheduled_reminder(app);
    }

    log::info!(
        "devicesync: auto-stopped '{}' ({hours:.2}h)",
        result.task_name
    );
    let _ = app.emit("timer_auto_stopped", &result);
    Some(result)
}

// ── Reconciler loop ──────────────────────────────────────────────────

fn current_client(app: &AppHandle) -> Option<OdooClient> {
    let state = app.state::<AppState>();
    let odoo = state.odoo.lock().unwrap();
    odoo.clone()
}

/// Foreground when the window is on screen; the desktop app hides to the tray,
/// and there is no point polling every 15s for a UI nobody is looking at.
fn window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(true)
}

/// One reconcile pass.
///
/// `exchange` is false when sync has been switched off: finished runs still
/// have to reach Odoo — their time was already tracked and would otherwise be
/// stranded in the outbox — but nothing new is published or adopted.
/// Returns whether the pass did anything, which drives the poll backoff.
async fn reconcile(
    app: &AppHandle,
    client: &OdooClient,
    exchange: bool,
    last_heartbeat: &mut Option<(String, Instant)>,
    missing_polls: &mut u32,
    net_failures: &mut u32,
) -> bool {
    let (lock, my_device) = {
        let state = app.state::<AppState>();
        let device_id = state.device.lock().unwrap().id.clone();
        (state.sync_lock.clone(), device_id)
    };
    let _guard = lock.lock().await;

    let mut changed = flush_outbox(app, client).await;
    if !exchange {
        return changed;
    }

    // Pull before push: the push needs to know whether our line is still live
    // in Odoo before it writes anything to it.
    let pulled = pull(app, client, missing_polls, net_failures).await;
    changed |= pulled.changed;
    changed |= push(app, client, &my_device, pulled.ours_is_live, last_heartbeat).await;
    // `pull` may have queued losing sessions; settle them without waiting a
    // whole poll interval, so their markers stop looking live to other devices.
    changed |= flush_outbox(app, client).await;
    changed
}

/// Back off while the reconciler has nothing to do: doubling per quiet pass,
/// from the base cadence up to a ceiling. Any activity — or a `nudge()` from a
/// timer command or window focus — resets it to the base.
fn poll_interval(visible: bool, quiet_passes: u32) -> u64 {
    let (base, cap) = if visible {
        (POLL_FOREGROUND_SECS, POLL_FOREGROUND_MAX_SECS)
    } else {
        (POLL_BACKGROUND_SECS, POLL_BACKGROUND_MAX_SECS)
    };
    (base << quiet_passes.min(4)).min(cap)
}

/// See [`NET_FAILURE_BASE_SECS`]: doubling per consecutive failed pass.
fn net_failure_backoff(failures: u32) -> u64 {
    (NET_FAILURE_BASE_SECS << failures.saturating_sub(1).min(4)).min(NET_FAILURE_MAX_SECS)
}

/// First consecutive failure, then only every [`FAILURE_LOG_EVERY`]th.
fn should_log_failure(failures: u32) -> bool {
    failures == 1 || failures % FAILURE_LOG_EVERY == 0
}

pub async fn run_sync_loop(app: AppHandle) {
    use tokio::time::{sleep, Duration as TokioDuration};

    // Give auto-login a chance to land first.
    sleep(TokioDuration::from_secs(12)).await;

    let mut last_heartbeat: Option<(String, Instant)> = None;
    let mut missing_polls: u32 = 0;
    let mut last_recents: Option<Instant> = None;
    let mut quiet_passes: u32 = 0;
    let mut net_failures: u32 = 0;

    loop {
        let exchange = sync_enabled(&app);
        if let Some(client) = current_client(&app) {
            let changed = reconcile(
                &app,
                &client,
                exchange,
                &mut last_heartbeat,
                &mut missing_polls,
                &mut net_failures,
            )
            .await;
            quiet_passes = if changed { 0 } else { quiet_passes.saturating_add(1) };

            let recents_due = last_recents
                .map(|at| at.elapsed() >= Duration::from_secs(RECENTS_REFRESH_SECS))
                .unwrap_or(true);
            if exchange && recents_due && net_failures == 0 {
                pull_recent_tasks(&app, &client).await;
                last_recents = Some(Instant::now());
            }
        }

        let mut interval = poll_interval(window_visible(&app), quiet_passes);
        // Mobile only: while Odoo is unreachable (Android's background network
        // policy blocks the app outright), stretch the doomed retries instead
        // of hammering. Desktop keeps its normal cadence.
        if cfg!(mobile) && net_failures > 0 {
            interval = interval.max(net_failure_backoff(net_failures));
        }
        let wakeup = app.state::<AppState>().sync_wakeup.clone();
        tokio::select! {
            _ = sleep(TokioDuration::from_secs(interval)) => {}
            // Someone asked for a pass now (timer command, window focus / app
            // resume) — treat it as activity so the next few passes stay
            // responsive, and give the network the benefit of the doubt.
            _ = wakeup.notified() => {
                quiet_passes = 0;
                net_failures = 0;
            }
        }
    }
}
