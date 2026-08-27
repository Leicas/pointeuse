use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(desktop)]
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tokio::time::{self, Duration};

use crate::commands::settings::QuickSwitchItem;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// ReminderState
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ReminderState {
    pub interval_minutes: u64,
    pub popup_showing: bool,
    /// ID of the currently scheduled reminder task (mobile only)
    pub scheduled_task_id: Option<String>,
    /// Set to true when a task switch occurs; the reminder loop resets its counter
    pub reset_elapsed: bool,
}

// ---------------------------------------------------------------------------
// Event payload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickSwitchEntry {
    pub task_id: i64,
    pub task_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub slot: String, // "main" or "small"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderPayload {
    pub task_id: i64,
    pub task_name: String,
    pub project_name: String,
    pub elapsed_secs: u64,
    pub quick_switch: Vec<QuickSwitchEntry>,
}

// ---------------------------------------------------------------------------
// Popup window (desktop only)
// ---------------------------------------------------------------------------

/// Show the reminder as a small always-on-top popup window.
#[cfg(desktop)]
pub fn show_reminder_window(app: &AppHandle, payload: &ReminderPayload) {
    // If window already exists, focus it
    if let Some(win) = app.get_webview_window("reminder") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }

    let task_name = payload.task_name.replace('\'', "\\'").replace('\n', " ");
    let task_name_short = if task_name.len() > 30 {
        format!("{}...", &task_name[..27])
    } else {
        task_name.clone()
    };
    let project_name = payload.project_name.replace('\'', "\\'").replace('\n', " ");
    let elapsed = payload.elapsed_secs;
    let h = elapsed / 3600;
    let m = (elapsed % 3600) / 60;
    let s = elapsed % 60;
    let time_str = format!("{:02}:{:02}:{:02}", h, m, s);

    // Build quick-switch items (skip currently running task)
    let qs_items: Vec<&QuickSwitchEntry> = payload
        .quick_switch
        .iter()
        .filter(|qs| qs.task_id != payload.task_id)
        .collect();

    let mut qs_main_html = String::new();
    let mut qs_small_html = String::new();
    for qs in &qs_items {
        let name = qs.task_name.replace('\'', "\\'").replace('\n', " ");
        let proj_qs = qs.project_name.replace('\'', "\\'").replace('\n', " ");
        let tid = qs.task_id;
        let pid = qs.project_id;
        if qs.slot == "main" {
            qs_main_html.push_str(&format!(
                r#"<div class="qs-row" onclick="quickSwitch({tid},'{name}',{pid},'{proj_qs}')"><div class="qs-body"><div class="qs-name">{name}</div><div class="qs-proj">{proj_qs}</div></div><svg class="qs-arrow" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg></div>"#
            ));
        } else {
            qs_small_html.push_str(&format!(
                r#"<div class="qs-pill" onclick="quickSwitch({tid},'{name}',{pid},'{proj_qs}')" title="{proj_qs}">{name}</div>"#
            ));
        }
    }

    let has_quickswitch = !qs_main_html.is_empty() || !qs_small_html.is_empty();

    let qs_section = if has_quickswitch {
        let mut s = String::from(r#"<div class="qs"><div class="qs-head"><span class="qs-title">Switch to</span><div class="qs-all" onclick="switchTask()">All tasks &#8250;</div></div>"#);
        if !qs_main_html.is_empty() {
            s.push_str(&format!(r#"<div class="qs-list">{qs_main_html}</div>"#));
        }
        if !qs_small_html.is_empty() {
            s.push_str(&format!(r#"<div class="qs-pills">{qs_small_html}</div>"#));
        }
        s.push_str("</div>");
        s
    } else {
        String::new()
    };

    let main_count = qs_items.iter().filter(|q| q.slot == "main").count();
    let has_pills = qs_items.iter().any(|q| q.slot != "main");
    let popup_height = if has_quickswitch {
        // 2-line rows are ~52px each, pills row ~34px, header ~28px, top section ~210px
        210.0 + 28.0 + (main_count as f64 * 52.0) + if has_pills { 34.0 } else { 0.0 }
    } else {
        220.0
    };

    let html = format!(r#"
<!DOCTYPE html>
<html><head><meta charset="UTF-8"/>
<script>try{{const t=localStorage.getItem('pointeuse-theme');if(t&&t!=='dark')document.documentElement.setAttribute('data-theme',t)}}catch(_){{}}</script>
<style>
:root{{
  --p-bg:#0f1219;--p-text:#c8cdd8;--p-heading:#e8ecf4;--p-muted:#7a8194;
  --p-brand:#3b82f6;--p-brand-deep:#1d4ed8;--p-brand-bright:#7ab8ff;
  --p-danger:#f87171;--p-danger-bg:#1a1220;--p-danger-border:rgba(248,113,113,.2);--p-danger-hover-bg:#251a22;--p-danger-hover-border:rgba(248,113,113,.4);
  --p-border:#1e2235;--p-qs-lbl:#4b83ee;--p-qs-border:#252a3d;--p-qs-hover:#141924;--p-qs-active:#1a2030;
  --p-name:#dde1ea;--p-proj:#444d68;--p-arrow:#2d3450;
}}
[data-theme="light"]{{
  --p-bg:#f5f6fa;--p-text:#4b5068;--p-heading:#1a1d2e;--p-muted:#6b7190;
  --p-brand:#2563eb;--p-brand-deep:#1e40af;--p-brand-bright:#1d4ed8;
  --p-danger:#dc2626;--p-danger-bg:#fef2f2;--p-danger-border:rgba(220,38,38,.2);--p-danger-hover-bg:#fee2e2;--p-danger-hover-border:rgba(220,38,38,.4);
  --p-border:#d4d7e2;--p-qs-lbl:#2563eb;--p-qs-border:#d4d7e2;--p-qs-hover:#e8eaf2;--p-qs-active:#dfe1ec;
  --p-name:#1a1d2e;--p-proj:#6b7190;--p-arrow:#9ba2b8;
}}
[data-theme="colorblind"]{{
  --p-danger:#fb923c;--p-danger-bg:#1a1510;--p-danger-border:rgba(234,88,12,.2);--p-danger-hover-bg:#251c12;--p-danger-hover-border:rgba(234,88,12,.4);
}}
*{{margin:0;padding:0;box-sizing:border-box}}
body{{
  font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;
  background:var(--p-bg);color:var(--p-text);
  display:flex;flex-direction:column;align-items:center;
  height:100vh;padding:14px;user-select:none;
  overflow:hidden;
}}
.top{{text-align:center;margin-bottom:8px}}
.bell{{color:var(--p-brand);margin-bottom:4px}}
h2{{font-size:14px;font-weight:700;color:var(--p-heading);margin-bottom:2px}}
.info{{font-size:11px;color:var(--p-muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:260px}}
.clock{{font-size:20px;font-weight:700;font-family:"SF Mono","Cascadia Code",monospace;color:var(--p-brand-bright);margin:4px 0 8px}}
.btns{{display:flex;gap:6px}}
.btns button{{padding:7px 18px;font-size:11px;font-weight:600;border:none;border-radius:7px;cursor:pointer;transition:all .12s}}
.b-stop{{background:var(--p-danger-bg);color:var(--p-danger);border:1px solid var(--p-danger-border)}}
.b-stop:hover{{background:var(--p-danger-hover-bg);border-color:var(--p-danger-hover-border)}}
.b-keep{{background:linear-gradient(135deg,var(--p-brand-deep),var(--p-brand));color:#fff;box-shadow:0 2px 8px rgba(59,130,246,.25)}}
.b-keep:hover{{box-shadow:0 3px 14px rgba(59,130,246,.4)}}

.qs{{width:100%;margin-top:8px;padding-top:8px;border-top:1px solid var(--p-border);flex:1;overflow:hidden}}
.qs-head{{display:flex;justify-content:space-between;align-items:center;margin-bottom:6px}}
.qs-lbl{{font-size:9px;font-weight:700;color:var(--p-qs-lbl);text-transform:uppercase;letter-spacing:.6px}}
.qs-all{{font-size:9px;font-weight:600;color:var(--p-muted);cursor:pointer;padding:2px 8px;border-radius:5px;border:1px solid var(--p-qs-border);transition:all .12s}}
.qs-all:hover{{color:var(--p-heading);border-color:var(--p-brand);background:var(--p-qs-hover)}}

.qs-list{{display:flex;flex-direction:column;gap:1px;margin-bottom:4px}}
.qs-row{{
  display:flex;align-items:flex-start;gap:0;
  padding:8px 10px;border-radius:8px;cursor:pointer;
  border-left:3px solid transparent;
  transition:all .1s;position:relative;
}}
.qs-row:hover{{background:var(--p-qs-hover);border-left-color:var(--p-brand)}}
.qs-row:active{{background:var(--p-qs-active)}}
.qs-body{{flex:1;min-width:0}}
.qs-name{{
  font-size:11.5px;font-weight:600;color:var(--p-name);line-height:1.35;
  display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;
  overflow:hidden;word-break:break-word;
}}
.qs-proj{{font-size:9px;color:var(--p-proj);margin-top:1px}}
.qs-arrow{{color:var(--p-arrow);margin-left:6px;margin-top:4px;flex-shrink:0;transition:color .1s}}
.qs-row:hover .qs-arrow{{color:var(--p-brand)}}

.qs-pills{{display:flex;gap:4px;flex-wrap:wrap}}
.qs-pill{{
  padding:5px 10px;font-size:9.5px;font-weight:600;
  color:var(--p-muted);border:1px solid var(--p-qs-border);border-radius:6px;
  cursor:pointer;transition:all .1s;
  overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:130px;
}}
.qs-pill:hover{{color:var(--p-name);border-color:var(--p-brand);background:var(--p-qs-hover)}}
</style>
</head><body>
<div class="top">
  <div class="bell"><svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg></div>
  <h2>Still working on {task_name_short}?</h2>
  <div class="info">{task_name}{proj}</div>
  <div class="clock">{time_str}</div>
  <div class="btns">
    <button class="b-stop" onclick="stopTimer()">Stop Timer</button>
    <button class="b-keep" onclick="keepGoing()">Keep Going</button>
  </div>
</div>
{qs_section}
<script>
const invoke=window.__TAURI__.core.invoke;
async function showMain(){{try{{const w=window.__TAURI__.window.Window.getByLabel('main');if(w){{await w.show();await w.setFocus()}}}}catch(_){{}}}}
async function keepGoing(){{try{{await invoke('dismiss_idle_reminder')}}catch(_){{}}window.__TAURI__.window.getCurrentWindow().close()}}
async function switchTask(){{
  try{{await invoke('dismiss_idle_reminder')}}catch(_){{}}
  await showMain();
  try{{await window.__TAURI__.event.emit('open_task_picker',{{mode:'switch'}})}}catch(_){{}}
  // Small delay so the main window receives the event before we close
  setTimeout(()=>window.__TAURI__.window.getCurrentWindow().close(),80);
}}
async function stopTimer(){{
  try{{
    await invoke('dismiss_idle_reminder');
    const s=await invoke('stop_timer');
    if(s&&s.elapsed_secs>0){{
      const h=s.elapsed_secs/3600;
      const d=new Date(Date.now()-new Date().getTimezoneOffset()*60000).toISOString().slice(0,10);
      await invoke('log_time',{{taskId:s.task_id,projectId:s.project_id||0,taskName:s.task_name,projectName:s.project_name||'',description:s.task_name,durationHours:h,date:d}});
    }}
  }}catch(e){{console.error('stopTimer error:',e)}}
  try{{await window.__TAURI__.event.emit('reminder_timer_logged',{{}})}}catch(_){{}}
  await showMain();
  window.__TAURI__.window.getCurrentWindow().close();
}}
async function quickSwitch(t,n,p,pn){{
  try{{
    await invoke('dismiss_idle_reminder');
    const s=await invoke('stop_timer');
    if(s&&s.elapsed_secs>0){{const h=s.elapsed_secs/3600;const d=new Date(Date.now()-new Date().getTimezoneOffset()*60000).toISOString().slice(0,10);await invoke('log_time',{{taskId:s.task_id,projectId:s.project_id||0,taskName:s.task_name,projectName:s.project_name||'',description:s.task_name,durationHours:h,date:d}})}}
    await invoke('start_timer',{{taskId:t,taskName:n,projectId:p,projectName:pn}});
  }}catch(e){{console.error(e)}}
  try{{await window.__TAURI__.event.emit('reminder_timer_logged',{{}})}}catch(_){{}}
  await showMain();
  window.__TAURI__.window.getCurrentWindow().close();
}}
</script>
</body></html>
"#,
        proj = if project_name.is_empty() { String::new() } else { format!(" &middot; {}", project_name) }
    );

    // Use an inline HTML page via a data-like approach
    // We'll create the window pointing to the main frontend, then replace content
    let url = WebviewUrl::App("index.html".into());

    let builder = WebviewWindowBuilder::new(app, "reminder", url)
        .title("Pointeuse — Reminder")
        .inner_size(300.0, popup_height)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .center()
        .focused(true);

    match builder.build() {
        Ok(win) => {
            let escaped = html.replace('\\', "\\\\").replace('`', "\\`");
            let js = format!("document.open();document.write(`{}`);document.close();", escaped);
            // Small delay to let the webview initialize before eval
            let win_clone = win.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if let Err(e) = win_clone.eval(&js) {
                    log::error!("Failed to eval reminder HTML: {e}");
                }
            });
            log::info!("Reminder popup window created");
        }
        Err(e) => log::error!("Failed to build reminder window: {e}"),
    }
}

/// On mobile, show a system notification with action buttons (Continue / Change Task / Stop).
/// No in-app overlay — the notification handles everything.
#[cfg(mobile)]
pub fn show_reminder_window(app: &AppHandle, payload: &ReminderPayload) {
    crate::notification::show_reminder_notification(
        app,
        &payload.task_name,
        &payload.project_name,
        payload.elapsed_secs,
    );
    log::info!("Reminder notification shown (mobile)");
}

// ---------------------------------------------------------------------------
// Quick-switch entry builder
// ---------------------------------------------------------------------------

/// Public wrapper for test_reminder_popup command
pub async fn build_quick_switch_entries_public(app: &AppHandle) -> Vec<QuickSwitchEntry> {
    build_quick_switch_entries(app).await
}

async fn build_quick_switch_entries(app: &AppHandle) -> Vec<QuickSwitchEntry> {
    use crate::commands::settings::DefaultTaskConfig;
    use tauri_plugin_store::StoreExt;

    // Load default task (if configured) to prepend to entries
    let default_task: Option<DefaultTaskConfig> = app
        .store("settings.json")
        .ok()
        .and_then(|s| s.get("default_task"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let mode = app
        .store("settings.json")
        .ok()
        .and_then(|s| s.get("quickswitch_mode"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "auto".to_string());

    let mut entries = Vec::new();

    // Prepend default task as first "main" entry if configured
    if let Some(dt) = &default_task {
        entries.push(QuickSwitchEntry {
            task_id: dt.task_id,
            task_name: format!("\u{1F512} {}", dt.task_name),
            project_id: dt.project_id,
            project_name: dt.project_name.clone(),
            slot: "main".to_string(),
        });
    }

    if mode == "manual" {
        // Load user-pinned items from settings
        let items: Vec<QuickSwitchItem> = app
            .store("settings.json")
            .ok()
            .and_then(|s| s.get("quickswitch_items"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        entries.extend(items.into_iter().filter(|i| {
            // Skip if same as default task
            default_task.as_ref().map_or(true, |dt| dt.task_id != i.task_id)
        }).map(|i| QuickSwitchEntry {
            task_id: i.task_id,
            task_name: i.task_name,
            project_id: i.project_id,
            project_name: i.project_name,
            slot: i.slot,
        }));

        return entries;
    }

    // Auto mode: pull from recent tasks / heuristics
    let state = app.state::<AppState>();
    let recent = {
        let db = state.db.lock().unwrap();
        crate::db::tasks::get_recent_tasks(&db, 7).unwrap_or_default()
    };

    for (i, t) in recent.into_iter().enumerate() {
        // Skip if same as default task
        if default_task.as_ref().is_some_and(|dt| dt.task_id == t.id) {
            continue;
        }
        let slot = if i < 4 { "main" } else { "small" };
        entries.push(QuickSwitchEntry {
            task_id: t.id,
            task_name: t.name,
            project_id: t.project_id.unwrap_or(0),
            project_name: t.project_name.unwrap_or_default(),
            slot: slot.to_string(),
        });
    }
    entries
}

// ---------------------------------------------------------------------------
// Background reminder loop (runs on both desktop and mobile)
// ---------------------------------------------------------------------------

const TICK_SECS: u64 = 30;

pub async fn run_reminder_loop(app_handle: AppHandle) {
    let mut elapsed_since_last_reminder: u64 = 0;

    log::info!("[reminder] Background reminder loop started (tick={}s)", TICK_SECS);

    loop {
        time::sleep(Duration::from_secs(TICK_SECS)).await;

        let state = app_handle.state::<AppState>();

        let (interval_minutes, popup_showing, should_reset) = {
            let mut reminder = state.reminder.lock().unwrap();
            let reset = reminder.reset_elapsed;
            if reset {
                reminder.reset_elapsed = false;
            }
            (reminder.interval_minutes, reminder.popup_showing, reset)
        };

        if should_reset {
            log::info!("[reminder] Resetting elapsed counter (task switch)");
            elapsed_since_last_reminder = 0;
        }

        if interval_minutes == 0 {
            elapsed_since_last_reminder = 0;
            continue;
        }

        let timer_info = {
            let timer = state.timer.lock().unwrap();
            timer.get_state()
        };

        if !timer_info.is_running {
            elapsed_since_last_reminder = 0;
            continue;
        }

        elapsed_since_last_reminder += TICK_SECS;

        let interval_secs = interval_minutes * 60;
        log::debug!(
            "[reminder] tick: elapsed={}s / {}s, popup_showing={}, task={:?}",
            elapsed_since_last_reminder,
            interval_secs,
            popup_showing,
            timer_info.task_name
        );
        if elapsed_since_last_reminder < interval_secs {
            continue;
        }

        if popup_showing {
            continue;
        }

        log::info!(
            "[reminder] Interval reached ({}m), showing reminder for task {:?}",
            interval_minutes,
            timer_info.task_name
        );

        {
            let mut reminder = state.reminder.lock().unwrap();
            reminder.popup_showing = true;
        }

        let quick_switch = build_quick_switch_entries(&app_handle).await;

        let payload = ReminderPayload {
            task_id: timer_info.task_id.unwrap_or(0),
            task_name: timer_info.task_name.unwrap_or_default(),
            project_name: timer_info.project_name.unwrap_or_default(),
            elapsed_secs: timer_info.elapsed_secs,
            quick_switch,
        };

        show_reminder_window(&app_handle, &payload);
        elapsed_since_last_reminder = 0;
    }
}

// ---------------------------------------------------------------------------
// Scheduled task handler (mobile — uses tauri-plugin-schedule-task)
// ---------------------------------------------------------------------------

/// Handler invoked by the schedule-task plugin when any scheduled task fires.
pub struct ScheduledTaskRouter;

impl tauri_plugin_schedule_task::ScheduledTaskHandler<tauri::Wry> for ScheduledTaskRouter {
    fn handle_scheduled_task(
        &self,
        task_name: &str,
        _parameters: HashMap<String, String>,
        app: &AppHandle,
    ) -> tauri_plugin_schedule_task::Result<()> {
        log::info!("[scheduler] handle_scheduled_task called, task_name='{}'", task_name);

        match task_name {
            "idle_reminder" => handle_idle_reminder(app),
            "attendance_check" => {
                let app_clone = app.clone();
                tauri::async_runtime::spawn(async move {
                    handle_attendance_check(&app_clone).await;
                });
                Ok(())
            }
            _ => {
                log::info!("[scheduler] Ignoring unknown task: {}", task_name);
                Ok(())
            }
        }
    }
}

fn handle_idle_reminder(app: &AppHandle) -> tauri_plugin_schedule_task::Result<()> {

        let state = app.state::<AppState>();

        // Check if timer is running
        let timer_info = {
            let timer = state.timer.lock().unwrap();
            timer.get_state()
        };

        log::info!(
            "[reminder] Timer state: running={}, elapsed={}s, task={:?}",
            timer_info.is_running,
            timer_info.elapsed_secs,
            timer_info.task_name
        );

        if !timer_info.is_running {
            log::info!("[reminder] Timer not running, skipping");
            let mut reminder = state.reminder.lock().unwrap();
            reminder.scheduled_task_id = None;
            return Ok(());
        }

        let (interval_minutes, popup_showing) = {
            let reminder = state.reminder.lock().unwrap();
            (reminder.interval_minutes, reminder.popup_showing)
        };

        log::info!("[reminder] Interval: {} minutes, popup_showing: {}", interval_minutes, popup_showing);

        if interval_minutes == 0 {
            log::info!("[reminder] Interval is 0, skipping");
            return Ok(());
        }

        // If the tokio loop already showed the reminder, just reschedule
        if popup_showing {
            log::info!("[reminder] Popup already showing (tokio loop beat us), rescheduling only");
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                schedule_next_reminder(&app_clone).await;
            });
            return Ok(());
        }

        // Mark popup as showing
        {
            let mut reminder = state.reminder.lock().unwrap();
            reminder.popup_showing = true;
        }

        let task_name_str = timer_info.task_name.unwrap_or_default();
        let project_name_str = timer_info.project_name.unwrap_or_default();

        // Show the reminder notification
        #[cfg(mobile)]
        {
            log::info!("[reminder] Calling show_reminder_notification...");
            crate::notification::show_reminder_notification(
                app,
                &task_name_str,
                &project_name_str,
                timer_info.elapsed_secs,
            );
        }

        // Emit event for in-app overlay
        let payload = ReminderPayload {
            task_id: timer_info.task_id.unwrap_or(0),
            task_name: task_name_str,
            project_name: project_name_str,
            elapsed_secs: timer_info.elapsed_secs,
            quick_switch: Vec::new(), // Quick-switch requires async; frontend will fetch if needed
        };
        log::info!("[reminder] Emitting show_idle_reminder event");
        let _ = app.emit("show_idle_reminder", &payload);

        // Schedule the next reminder
        log::info!("[reminder] Scheduling next reminder...");
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            schedule_next_reminder(&app_clone).await;
        });

    Ok(())
}

// ---------------------------------------------------------------------------
// Background attendance check (mobile — fires from scheduled task)
// ---------------------------------------------------------------------------

/// Base cadence of the scheduled attendance check.
const ATTENDANCE_BASE_SECS: u64 = 120;
/// Ceiling while Odoo stays unreachable. Android's background network policy
/// can block the app outright (DNS errors on every request), so consecutive
/// failures double the reschedule delay up to this instead of retrying at
/// full cadence; the first success resets it.
const ATTENDANCE_MAX_SECS: u64 = 600;
/// The failed retries are identical, so after the first one log only every Nth.
const ATTENDANCE_LOG_EVERY: u32 = 5;

/// Consecutive failed fetches. The check has no loop to hold state — each fire
/// comes fresh out of WorkManager — so the counter lives here.
static ATTENDANCE_FAILURES: AtomicU32 = AtomicU32::new(0);

/// Doubling per consecutive failure: 240s, 480s, then capped at 10 min.
fn attendance_backoff_secs(failures: u32) -> u64 {
    (ATTENDANCE_BASE_SECS << failures.min(3)).min(ATTENDANCE_MAX_SECS)
}

async fn handle_attendance_check(app: &AppHandle) {
    log::info!("[attendance] Scheduled attendance check fired");

    let client = {
        let state = app.state::<AppState>();
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.as_ref().cloned()
    };

    let client = match client {
        Some(c) => c,
        None => {
            log::info!("[attendance] Not logged in, skipping");
            schedule_attendance_check(app).await;
            return;
        }
    };

    let status = match client.get_attendance_status().await {
        Ok(s) => {
            ATTENDANCE_FAILURES.store(0, Ordering::Relaxed);
            s
        }
        Err(e) => {
            let failures = ATTENDANCE_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
            if failures == 1 || failures % ATTENDANCE_LOG_EVERY == 0 {
                log::error!("[attendance] Failed to fetch status ({failures}x in a row): {e}");
            }
            schedule_attendance_check_in(app, attendance_backoff_secs(failures)).await;
            return;
        }
    };

    let changed = {
        let state = app.state::<AppState>();
        let mut last = state.last_attendance.lock().unwrap();
        let changed = last.as_ref() != Some(&status);
        if changed {
            *last = Some(status.clone());
        }
        changed
    };

    if changed {
        log::info!("[attendance] Status changed: checked_in={}", status.is_checked_in);

        if !status.is_checked_in {
            // Auto-stop timer on external checkout. Settling it in Odoo and
            // clearing the ongoing notification is the helper's job.
            crate::devicesync::auto_stop_timer(app, &client).await;
        }

        let _ = app.emit("attendance_changed", &status);
    }

    // Schedule next check
    schedule_attendance_check(app).await;
}

/// Schedule the next attendance check via WorkManager (mobile only).
pub async fn schedule_attendance_check(app: &AppHandle) {
    schedule_attendance_check_in(app, ATTENDANCE_BASE_SECS).await;
}

async fn schedule_attendance_check_in(app: &AppHandle, delay_secs: u64) {
    use tauri_plugin_schedule_task::ScheduleTaskExt;

    let request = tauri_plugin_schedule_task::ScheduleTaskRequest {
        task_name: "attendance_check".to_string(),
        schedule_time: tauri_plugin_schedule_task::ScheduleTime::Duration(delay_secs),
        parameters: None,
    };

    match app.schedule_task().schedule_task(request).await {
        Ok(response) if response.success => {
            log::info!(
                "[attendance] Scheduled next check in {delay_secs}s (task_id: {})",
                response.task_id
            );
        }
        Ok(response) => {
            log::error!("[attendance] Schedule failed: {}", response.message.unwrap_or_default());
        }
        Err(e) => {
            log::error!("[attendance] Schedule error: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Schedule / cancel helpers (used on mobile)
// ---------------------------------------------------------------------------

/// Schedule the next reminder notification via the schedule-task plugin.
/// Cancels any existing scheduled reminder first.
pub async fn schedule_next_reminder(app: &AppHandle) {
    use tauri_plugin_schedule_task::ScheduleTaskExt;

    log::info!("[reminder] schedule_next_reminder called");

    let state = app.state::<AppState>();

    let (interval_minutes, old_task_id) = {
        let reminder = state.reminder.lock().unwrap();
        (reminder.interval_minutes, reminder.scheduled_task_id.clone())
    };

    log::info!(
        "[reminder] interval={}m, existing_task_id={:?}",
        interval_minutes,
        old_task_id
    );

    // Cancel existing scheduled reminder if any
    if let Some(task_id) = old_task_id {
        let cancel_req = tauri_plugin_schedule_task::CancelTaskRequest {
            task_id: task_id.clone(),
        };
        match app.schedule_task().cancel_task(cancel_req) {
            Ok(_) => log::info!("[reminder] Cancelled previous task: {}", task_id),
            Err(e) => log::error!("[reminder] Failed to cancel previous task {}: {e}", task_id),
        }
    }

    if interval_minutes == 0 {
        log::info!("[reminder] Interval is 0, not scheduling");
        let mut reminder = state.reminder.lock().unwrap();
        reminder.scheduled_task_id = None;
        return;
    }

    // Check timer is running
    let is_running = {
        let timer = state.timer.lock().unwrap();
        timer.is_running()
    };

    if !is_running {
        log::info!("[reminder] Timer not running, not scheduling");
        let mut reminder = state.reminder.lock().unwrap();
        reminder.scheduled_task_id = None;
        return;
    }

    let duration_secs = interval_minutes * 60;
    log::info!(
        "[reminder] Scheduling task 'idle_reminder' with duration={}s ({}m)",
        duration_secs,
        interval_minutes
    );

    let request = tauri_plugin_schedule_task::ScheduleTaskRequest {
        task_name: "idle_reminder".to_string(),
        schedule_time: tauri_plugin_schedule_task::ScheduleTime::Duration(duration_secs),
        parameters: None,
    };

    match app.schedule_task().schedule_task(request).await {
        Ok(response) if response.success => {
            log::info!(
                "[reminder] Scheduled OK in {}m (task_id: {})",
                interval_minutes,
                response.task_id
            );
            let mut reminder = state.reminder.lock().unwrap();
            reminder.scheduled_task_id = Some(response.task_id);
        }
        Ok(response) => {
            log::error!(
                "[reminder] Schedule FAILED: success=false, message={}",
                response.message.unwrap_or_else(|| "none".to_string())
            );
        }
        Err(e) => {
            log::error!("[reminder] Schedule ERROR: {e}");
        }
    }
}

/// Cancel any pending scheduled reminder.
#[allow(dead_code)]
pub fn cancel_scheduled_reminder(app: &AppHandle) {
    use tauri_plugin_schedule_task::ScheduleTaskExt;

    let state = app.state::<AppState>();
    let task_id = {
        let mut reminder = state.reminder.lock().unwrap();
        reminder.scheduled_task_id.take()
    };

    if let Some(task_id) = task_id {
        log::info!("[reminder] Cancelling scheduled reminder: {}", task_id);
        let cancel_req = tauri_plugin_schedule_task::CancelTaskRequest {
            task_id: task_id.clone(),
        };
        match app.schedule_task().cancel_task(cancel_req) {
            Ok(_) => log::info!("[reminder] Cancelled OK: {}", task_id),
            Err(e) => log::error!("[reminder] Cancel failed for {}: {e}", task_id),
        }
    } else {
        log::info!("[reminder] No scheduled reminder to cancel");
    }
}

#[cfg(test)]
mod attendance_backoff_tests {
    use super::*;

    #[test]
    fn failed_checks_back_off_to_a_ceiling() {
        assert_eq!(attendance_backoff_secs(0), 120, "base cadence untouched");
        assert_eq!(attendance_backoff_secs(1), 240);
        assert_eq!(attendance_backoff_secs(2), 480);
        assert_eq!(attendance_backoff_secs(3), 600);
        assert_eq!(attendance_backoff_secs(99), 600, "must stay capped");
    }
}
