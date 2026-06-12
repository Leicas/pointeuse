# Architecture Spec: Attendance Integration & Idle Reminder

This document specifies the exact changes needed to add Odoo attendance (check in / check out) support and an idle reminder system to the time-tracking app.

---

## A. Odoo Attendance Integration

### A.1 The `hr.attendance` Model

Odoo's `hr.attendance` model tracks employee presence. Key fields:

| Field | Type | Description |
|---|---|---|
| `id` | integer | Primary key |
| `employee_id` | many2one (hr.employee) | The employee. Returned as `[id, "Name"]` by `search_read` |
| `check_in` | datetime (string) | When the employee checked in. Format: `"2026-03-25 14:30:00"` (UTC, no timezone suffix) |
| `check_out` | datetime / False | When they checked out. `False` (boolean) while still checked in |
| `worked_hours` | float | Computed by Odoo. Hours between check_in and check_out. Only meaningful after check_out is set |

**How "currently checked in" works:** An employee is checked in if and only if there exists an `hr.attendance` record where `employee_id` matches AND `check_out` equals `False`. There is at most one such record per employee at any time.

### A.2 XML-RPC Calls

All calls go through the existing `execute_kw` helper on `OdooClient`, which wraps `/xmlrpc/2/object` -> `execute_kw`.

#### A.2.1 Get Current Attendance Status

**Purpose:** Determine if the current user's employee is checked in, and if so, when they checked in.

```
Model:    "hr.attendance"
Method:   "search_read"
Domain:   [
            ["employee_id", "=", <employee_id>],
            ["check_out", "=", false]
          ]
Fields:   ["id", "check_in"]
Limit:    1
```

- `employee_id` comes from `OdooClient::employee_id()` (already resolved during `connect()`).
- If result is empty array: employee is **not** checked in.
- If result has one record: employee **is** checked in. The record's `id` is the open attendance ID, and `check_in` is the start time.

**Domain encoding in Rust (`XmlRpcValue`):**
```rust
vec![
    XmlRpcValue::Array(vec![
        XmlRpcValue::String("employee_id".into()),
        XmlRpcValue::String("=".into()),
        XmlRpcValue::Int(employee_id),
    ]),
    XmlRpcValue::Array(vec![
        XmlRpcValue::String("check_out".into()),
        XmlRpcValue::String("=".into()),
        XmlRpcValue::Bool(false),
    ]),
]
```

#### A.2.2 Check In

**Purpose:** Create a new `hr.attendance` record with `check_in` set to now (UTC).

```
Model:    "hr.attendance"
Method:   "create"
Values:   {
            "employee_id": <employee_id>,
            "check_in": "2026-03-25 14:30:00"
          }
```

**Datetime format:** Odoo expects UTC datetime as `"YYYY-MM-DD HH:MM:SS"` with no `T` separator and no `Z` suffix. In Rust:
```rust
let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
```

**Precondition:** Must verify no open attendance exists first (call A.2.1). If one exists, this is an error -- the user is already checked in.

**Returns:** The new record ID (i64).

#### A.2.3 Check Out

**Purpose:** Write `check_out = now` on the employee's open attendance record.

```
Model:    "hr.attendance"
Method:   "write"
IDs:      [<open_attendance_id>]
Values:   {
            "check_out": "2026-03-25 17:45:00"
          }
```

**Precondition:** Must first fetch the open attendance via A.2.1 to get the record ID. If no open record exists, this is an error -- the user is not checked in.

#### A.2.4 Get Today's Worked Hours (optional, for display)

**Purpose:** After checking out, show total hours worked today.

```
Model:    "hr.attendance"
Method:   "search_read"
Domain:   [
            ["employee_id", "=", <employee_id>],
            ["check_in", ">=", "<today_start_utc>"]
          ]
Fields:   ["worked_hours"]
Limit:    (none)
```

Where `<today_start_utc>` is `"2026-03-25 00:00:00"`. Sum up all `worked_hours` values.

### A.3 New Rust Functions in `odoo/client.rs`

Add these methods to `impl OdooClient`:

#### `get_attendance_status(&self) -> AppResult<AttendanceStatus>`

```rust
pub struct AttendanceStatus {
    pub is_checked_in: bool,
    pub attendance_id: Option<i64>,   // ID of the open hr.attendance record
    pub check_in_time: Option<String>, // UTC datetime string "YYYY-MM-DD HH:MM:SS"
}
```

- Requires `self.employee_id` to be `Some`. Return error if `None`.
- Calls `search_read("hr.attendance", domain, ["id", "check_in"], Some(1))` with domain from A.2.1.
- Parses the result into `AttendanceStatus`.

#### `check_in(&self) -> AppResult<i64>`

- Calls `get_attendance_status()` first. Error if already checked in.
- Formats `Utc::now()` as `"%Y-%m-%d %H:%M:%S"`.
- Calls `self.create("hr.attendance", values)` with `employee_id` and `check_in`.
- Returns the new attendance record ID.

#### `check_out(&self) -> AppResult<f64>`

- Calls `get_attendance_status()` first. Error if not checked in.
- Formats `Utc::now()` as `"%Y-%m-%d %H:%M:%S"`.
- Calls `self.write("hr.attendance", vec![attendance_id], values)` with `check_out`.
- Optionally fetches today's total worked hours (A.2.4) and returns as f64 hours.

### A.4 New Commands in `commands/attendance.rs`

#### `get_attendance_status`

```rust
#[tauri::command]
pub async fn get_attendance_status(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceStatus>
```

- Locks `state.odoo`, gets client reference, calls `client.get_attendance_status()`.
- Errors with `AppError::Auth` if not logged in.

#### `attendance_check_in`

```rust
#[tauri::command]
pub async fn attendance_check_in(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceCheckInResult>
```

Where `AttendanceCheckInResult` contains `{ attendance_id: i64, check_in_time: String }`.

- Locks `state.odoo`, calls `client.check_in()`.
- Updates `state.attendance` (see Section C).

#### `attendance_check_out`

```rust
#[tauri::command]
pub async fn attendance_check_out(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceCheckOutResult>
```

Where `AttendanceCheckOutResult` contains `{ worked_hours_today: f64 }`.

- Locks `state.odoo`, calls `client.check_out()`.
- Updates `state.attendance`.

### A.5 New Model File: `odoo/attendance.rs`

Define the serde-serializable structs:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceStatus {
    pub is_checked_in: bool,
    pub attendance_id: Option<i64>,
    pub check_in_time: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceCheckInResult {
    pub attendance_id: i64,
    pub check_in_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceCheckOutResult {
    pub worked_hours_today: f64,
}
```

---

## B. Idle Reminder System

### B.1 Overview

A configurable periodic reminder that fires while a timer is running, asking the user if they are still working on the current task. This is NOT a system notification -- it is a Tauri event that the frontend listens to and renders as an in-app popup/modal.

### B.2 Reminder Interval Options

Stored as an integer (minutes) in `tauri-plugin-store` under key `"reminder_interval_minutes"`.

| Value | Meaning |
|---|---|
| `0` | Off (no reminders) |
| `5` | Every 5 minutes |
| `10` | Every 10 minutes |
| `15` | Every 15 minutes |
| `30` | Every 30 minutes |
| `60` | Every 60 minutes |

Default: `0` (off).

### B.3 Background Task Architecture

Use a single `tokio::spawn`ed task that runs for the lifetime of the app. It does NOT need to be restarted when the interval changes.

**Algorithm:**

```
loop {
    sleep(1 minute)  // or a shorter granularity like 30 seconds

    // Read current config
    let interval = read reminder_interval from state
    if interval == 0 { continue }  // reminders disabled

    // Check if timer is running
    let timer_state = read timer state from state
    if !timer_state.is_running {
        reset elapsed-since-last-reminder to 0
        continue
    }

    // Check if enough time has passed since last reminder (or timer start)
    elapsed_since_last_reminder += sleep_duration
    if elapsed_since_last_reminder < interval { continue }

    // Check if a popup is already showing
    if popup_is_showing { continue }

    // Fire the reminder
    set popup_is_showing = true
    emit Tauri event "show_idle_reminder" with payload:
        { task_id, task_name, project_name, elapsed_secs }
    reset elapsed_since_last_reminder to 0
}
```

### B.4 Tauri Events

#### Backend -> Frontend: `show_idle_reminder`

Payload (JSON):
```json
{
    "task_id": 42,
    "task_name": "Fix login bug",
    "project_name": "Website Redesign",
    "elapsed_secs": 3600
}
```

The frontend displays a modal: "Still working on **Fix login bug**?" with two buttons:
- **Yes** -> calls `dismiss_idle_reminder` command
- **No** -> calls `stop_timer` command, then navigates to the time-log form

#### Frontend -> Backend (commands):

##### `dismiss_idle_reminder`

```rust
#[tauri::command]
pub async fn dismiss_idle_reminder(
    state: tauri::State<'_, AppState>,
) -> AppResult<()>
```

- Sets `popup_is_showing = false` in state.
- Resets the elapsed-since-last-reminder counter to 0 so the full interval restarts.

### B.5 Shared State for Reminder

The background task needs shared access to:
1. Whether the timer is running (already in `AppState::timer`)
2. The reminder interval (new)
3. Whether a popup is currently showing (new)
4. Elapsed time since last reminder was shown (internal to the background task, not in AppState)

Use `Arc<AtomicU64>` for the interval and `Arc<AtomicBool>` for the popup flag, so the background task can read them without locking a Mutex. Alternatively, use a dedicated `Mutex<ReminderState>`.

### B.6 New Commands

#### `set_reminder_interval`

```rust
#[tauri::command]
pub async fn set_reminder_interval(
    minutes: u64,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()>
```

- Validates `minutes` is one of `[0, 5, 10, 15, 30, 60]`.
- Updates `state.reminder.interval_minutes`.
- Persists to `tauri-plugin-store` under key `"reminder_interval_minutes"` in `settings.json`.

#### `get_reminder_interval`

```rust
#[tauri::command]
pub async fn get_reminder_interval(
    state: tauri::State<'_, AppState>,
) -> AppResult<u64>
```

- Returns current interval from state.

### B.7 Spawning the Background Task

In `lib.rs` inside the `.setup()` closure, after creating `AppState` and calling `app.manage(state)`:

```rust
// Spawn idle-reminder background loop
let app_handle = app.handle().clone();
tokio::spawn(async move {
    reminder::run_reminder_loop(app_handle).await;
});
```

The `run_reminder_loop` function reads state from `app_handle.state::<AppState>()` and emits events via `app_handle.emit("show_idle_reminder", payload)`.

### B.8 New Module: `reminder.rs` (or `reminder/mod.rs`)

Contains:
- `ReminderState` struct (interval, popup_showing flag)
- `run_reminder_loop(app_handle)` async function
- Idle reminder payload struct

---

## C. State Changes

### C.1 `AppState` (in `state.rs`)

```rust
use std::sync::Mutex;
use crate::odoo::client::OdooClient;
use crate::timer::engine::TimerEngine;
use crate::reminder::ReminderState;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub odoo: Mutex<Option<OdooClient>>,
    pub timer: Mutex<TimerEngine>,
    pub reminder: Mutex<ReminderState>,  // NEW
}
```

### C.2 `ReminderState`

```rust
pub struct ReminderState {
    pub interval_minutes: u64,    // 0 = off, 5/10/15/30/60
    pub popup_showing: bool,      // true while the reminder popup is displayed
}

impl Default for ReminderState {
    fn default() -> Self {
        Self {
            interval_minutes: 0,
            popup_showing: false,
        }
    }
}
```

### C.3 New Store Keys in `settings.json`

| Key | Type | Default | Description |
|---|---|---|---|
| `reminder_interval_minutes` | number | `0` | Idle reminder interval. 0 = off |

This is persisted via `tauri-plugin-store` alongside the existing `odoo_url`, `odoo_database`, `odoo_username` keys.

### C.4 Restoring Reminder Interval on Startup

In `lib.rs` `.setup()`, after creating the store, read `reminder_interval_minutes` and pass it into `ReminderState`:

```rust
let reminder_interval = app.store("settings.json")
    .ok()
    .and_then(|s| s.get("reminder_interval_minutes"))
    .and_then(|v| v.as_u64())
    .unwrap_or(0);

let state = AppState {
    db: Mutex::new(conn),
    odoo: Mutex::new(None),
    timer: Mutex::new(timer_engine),
    reminder: Mutex::new(ReminderState {
        interval_minutes: reminder_interval,
        popup_showing: false,
    }),
};
```

---

## D. Tray Updates

### D.1 Current Tray Structure

The tray currently has two items:
1. "Open" -- shows the main window
2. "Quit" -- exits the app

### D.2 New Tray Structure

```
┌──────────────────────────┐
│  ● Checked in (2h 15m)   │  <- status label (disabled, non-clickable)
│  Check Out                │  <- action item (or "Check In" if not checked in)
│  ─────────────────────── │
│  Open                     │
│  Quit                     │
└──────────────────────────┘
```

When **not** checked in:
```
┌──────────────────────────┐
│  ○ Not checked in         │  <- status label
│  Check In                 │
│  ─────────────────────── │
│  Open                     │
│  Quit                     │
└──────────────────────────┘
```

### D.3 Implementation Approach

The tray menu is currently built once in `setup_tray()`. To support dynamic attendance status:

1. **Store the tray icon handle** in `AppState` (or as a separate managed state) so it can be rebuilt.
2. **Add a `rebuild_tray` function** that:
   - Reads attendance status from `AppState`
   - Builds a new menu with the correct label and action
   - Calls `tray_icon.set_menu(Some(new_menu))`
3. **Call `rebuild_tray`** after:
   - Successful login (attendance status is fetched)
   - `attendance_check_in` command
   - `attendance_check_out` command
   - App startup (after auto-login)

### D.4 Tray Menu Event Handling

Add handlers for the new menu item IDs:

```rust
"attendance_toggle" => {
    // Read current attendance state
    // If checked in -> call check_out logic
    // If not checked in -> call check_in logic
    // Rebuild tray menu to reflect new state
}
```

The attendance toggle from tray should use the same `OdooClient` methods as the commands. Extract the core logic into shared functions that both the tray handler and the Tauri commands can call.

### D.5 Tray Tooltip Update

Update the tray tooltip to reflect attendance:
- Checked in: `"Time Tracker - Checked in since 9:30 AM"`
- Not checked in: `"Time Tracker"`

---

## E. Registration of New Commands

In `lib.rs`, add to the `invoke_handler`:

```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    commands::attendance::get_attendance_status,
    commands::attendance::attendance_check_in,
    commands::attendance::attendance_check_out,
    commands::reminder::set_reminder_interval,
    commands::reminder::get_reminder_interval,
    commands::reminder::dismiss_idle_reminder,
])
```

And in `commands/mod.rs`:
```rust
pub mod attendance;
pub mod reminder;
```

---

## F. Error Handling Notes

- All attendance commands must check that `state.odoo` is `Some` (user is logged in). Return `AppError::Auth("Not logged in")` otherwise.
- All attendance commands must check that `OdooClient::employee_id()` is `Some`. Return `AppError::Odoo("No employee record linked to this user")` otherwise.
- The `check_out` field being `False` in Odoo maps to `XmlRpcValue::Bool(false)` in the XML-RPC response. The domain filter `["check_out", "=", false]` must use `XmlRpcValue::Bool(false)`, not `XmlRpcValue::Nil` or an empty string.

---

## G. File Summary

| File | Action |
|---|---|
| `src-tauri/src/odoo/client.rs` | Add `get_attendance_status()`, `check_in()`, `check_out()` methods |
| `src-tauri/src/odoo/attendance.rs` | New file: `AttendanceStatus`, `AttendanceCheckInResult`, `AttendanceCheckOutResult` structs |
| `src-tauri/src/odoo/mod.rs` | Add `pub mod attendance;` |
| `src-tauri/src/commands/attendance.rs` | New file: `get_attendance_status`, `attendance_check_in`, `attendance_check_out` commands |
| `src-tauri/src/commands/reminder.rs` | New file: `set_reminder_interval`, `get_reminder_interval`, `dismiss_idle_reminder` commands |
| `src-tauri/src/commands/mod.rs` | Add `pub mod attendance;` and `pub mod reminder;` |
| `src-tauri/src/reminder.rs` | New file: `ReminderState`, `run_reminder_loop()`, idle reminder payload struct |
| `src-tauri/src/state.rs` | Add `reminder: Mutex<ReminderState>` field |
| `src-tauri/src/lib.rs` | Register new commands, spawn reminder task, restore reminder interval from store |
| `src-tauri/src/tray.rs` | Add attendance status display, check in/out toggle, `rebuild_tray()` function |
| `src-tauri/src/error.rs` | No changes needed (existing variants cover all cases) |
| `src/main.js` | Add `show_idle_reminder` event listener, reminder popup UI, attendance UI controls |
