// Pointeuse — Main Application
// Single-file frontend for Tauri v2

import {
  createEntryComposer, reconcileEntries, entryCapabilities,
  rowActionsHtml, rowAttrs, pendingBadgeHtml, pendingToRow,
} from './entry-composer.js';

function getInvoke() {
  if (window.__TAURI__?.core?.invoke) return window.__TAURI__.core.invoke;
  throw new Error('Tauri API not available');
}
// Wrap invoke to auto-show the global sync bar for any call taking > 150ms
const invoke = (...args) => {
  const raw = getInvoke();
  const promise = raw(...args);
  let shown = false;
  const timer = setTimeout(() => { shown = true; _showGlobalSync(); }, 150);
  const done = () => { clearTimeout(timer); if (shown) _hideGlobalSync(); };
  promise.then(done, done);
  return promise;
};

// ── State Store ───────────────────────────────────────────────────────

function createStore(initial) {
  let state = { ...initial };
  const listeners = new Set();
  return {
    getState: () => state,
    setState(partial) {
      const prev = state;
      state = { ...state, ...partial };
      listeners.forEach(fn => fn(state, prev));
    },
    subscribe(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    }
  };
}

const store = createStore({
  view: 'login',
  auth: { authenticated: false, username: null, url: null },
  timer: { is_running: false, task_id: null, task_name: null, project_id: null, project_name: null, elapsed_secs: 0 },
  attendance: { is_checked_in: false, attendance_id: null, check_in_time: null },
  tasks: [],
  recentTasks: [],
  myTasks: [],
  projects: [],
  todayEntries: [],
  syncStatus: { pending_count: 0 },
  stoppedTimer: null,
  reminderInterval: 0,
  quickswitchMode: 'auto', // 'auto' or 'manual'
  quickswitchItems: [],     // manual-mode pinned tasks
  hideDoneTasks: true,
  historyDate: null, // YYYY-MM-DD, null = today
  historyMode: 'day', // 'day' or 'month'
  monthYear: null, // { year, month } for month view
  todayPending: [],   // queued rows for the viewed date, never summed into the total
  justAddedKey: null, // "<task_id>|<hours>" — flashes the new row once
  device: { id: '', label: '' }, // this install's cross-device sync identity
  deviceSyncEnabled: true,
});

// ── API Layer ─────────────────────────────────────────────────────────

const api = {
  login: (url, database, username, password) => invoke('login', { url, database, username, password }),
  logout: () => invoke('logout'),
  checkAuth: () => invoke('check_auth'),
  getSavedConnection: () => invoke('get_saved_connection'),
  startTimer: (taskId, taskName, projectId, projectName) => invoke('start_timer', { taskId, taskName, projectId, projectName }),
  stopTimer: () => invoke('stop_timer'),
  discardTimer: () => invoke('discard_timer'),
  getTimerState: () => invoke('get_timer_state'),
  searchTasks: (query, projectId) => invoke('search_tasks', { query, projectId: projectId || null }),
  getMyTasks: () => invoke('get_my_tasks'),
  getRecentTasks: () => invoke('get_recent_tasks'),
  createTask: (name, projectId) => invoke('create_task', { name, projectId }),
  logTime: (taskId, projectId, taskName, projectName, description, durationHours, date) =>
    invoke('log_time', { taskId, projectId, taskName, projectName, description, durationHours, date }),
  getTodayEntries: () => invoke('get_today_entries'),
  getProjects: () => invoke('get_projects'),
  syncPending: () => invoke('sync_pending'),
  getSyncStatus: () => invoke('get_sync_status'),
  getPendingEntries: () => invoke('get_pending_entries'),
  getReviewEntries: () => invoke('get_review_entries'),
  resolveSyncEntry: (entryId, action) => invoke('resolve_sync_entry', { entryId, action }),
  retrySyncEntry: (entryId) => invoke('retry_sync_entry', { entryId }),
  // Attendance
  getAttendanceStatus: () => invoke('get_attendance_status'),
  attendanceCheckIn: () => invoke('attendance_check_in'),
  attendanceCheckOut: () => invoke('attendance_check_out'),
  // Reminder
  setReminderInterval: (minutes) => invoke('set_reminder_interval', { minutes }),
  getReminderInterval: () => invoke('get_reminder_interval'),
  dismissIdleReminder: () => invoke('dismiss_idle_reminder'),
  testReminderPopup: () => invoke('test_reminder_popup'),
  getReminderChannel: () => invoke('get_reminder_channel'),
  setReminderChannel: (channel) => invoke('set_reminder_channel', { channel }),
  // Autostart
  getAutostartEnabled: () => invoke('get_autostart_enabled'),
  setAutostartEnabled: (enabled) => invoke('set_autostart_enabled', { enabled }),
  // Analysis
  getDayAnalysis: (date) => invoke('get_day_analysis', { date }),
  // Task stages
  getTaskStages: (taskId, projectId) => invoke('get_task_stages', { taskId, projectId }),
  updateTaskStage: (taskId, stageId) => invoke('update_task_stage', { taskId, stageId }),
  updateTaskKanbanState: (taskId, kanbanState) => invoke('update_task_kanban_state', { taskId, kanbanState }),
  updateTaskState: (taskId, newState) => invoke('update_task_state', { taskId, newState }),
  // Smart suggestions
  getSuggestedTasks: () => invoke('get_suggested_tasks'),
  // History
  getEntriesForDate: (date) => invoke('get_entries_for_date', { date }),
  getMonthlySummary: (year, month) => invoke('get_monthly_summary', { year, month }),
  // Updater
  checkForUpdate: () => invoke('check_for_update'),
  installUpdate: () => invoke('install_update'),
  // Settings
  getQuickswitchMode: () => invoke('get_quickswitch_mode'),
  setQuickswitchMode: (mode) => invoke('set_quickswitch_mode', { mode }),
  getQuickswitchItems: () => invoke('get_quickswitch_items'),
  getQuickSwitchEntries: () => invoke('get_quick_switch_entries'),
  setQuickswitchItems: (items) => invoke('set_quickswitch_items', { items }),
  getHideDoneTasks: () => invoke('get_hide_done_tasks'),
  setHideDoneTasks: (hide) => invoke('set_hide_done_tasks', { hide }),
  // Default task
  getDefaultTask: () => invoke('get_default_task'),
  setDefaultTask: (taskId, taskName, projectId, projectName) => invoke('set_default_task', { taskId, taskName, projectId, projectName }),
  clearDefaultTask: () => invoke('clear_default_task'),
  // Manual timesheet entry
  preflightManualEntry: (taskId, projectId, durationHours, date, excludeOdooId) =>
    invoke('preflight_manual_entry', { taskId, projectId, durationHours, date, excludeOdooId: excludeOdooId ?? null }),
  createManualEntry: (taskId, projectId, taskName, projectName, description, durationHours, date, allowDuplicate) =>
    invoke('create_manual_entry', { taskId, projectId, taskName, projectName, description, durationHours, date, allowDuplicate }),
  updateTimesheetEntry: (odooId, taskId, projectId, taskName, projectName, description, durationHours, date, originalDate) =>
    invoke('update_timesheet_entry', { odooId, taskId, projectId, taskName, projectName, description, durationHours, date, originalDate }),
  deleteTimesheetEntry: (odooId, taskId, date) => invoke('delete_timesheet_entry', { odooId, taskId, date }),
  updatePendingEntry: (entryId, taskId, projectId, taskName, projectName, description, durationHours, date, allowDuplicate) =>
    invoke('update_pending_entry', { entryId, taskId, projectId, taskName, projectName, description, durationHours, date, allowDuplicate }),
  getPendingForDate: (date) => invoke('get_pending_for_date', { date }),
  // Cross-device sync
  getDeviceIdentity: () => invoke('get_device_identity'),
  syncDevicesNow: () => invoke('sync_devices_now'),
  getDeviceSyncEnabled: () => invoke('get_device_sync_enabled'),
  setDeviceSyncEnabled: (enabled) => invoke('set_device_sync_enabled', { enabled }),
  setDeviceLabel: (label) => invoke('set_device_label', { label }),
};

// ── Helpers ───────────────────────────────────────────────────────────

function formatTime(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

function formatHours(hours) {
  const abs = Math.abs(hours);
  const sign = hours < 0 ? '-' : '';
  const h = Math.floor(abs);
  const m = Math.round((abs - h) * 60);
  return `${sign}${h}h ${m}m`;
}

function todayDate() {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function $(sel) { return document.querySelector(sel); }
function $$(sel) { return document.querySelectorAll(sel); }

function addDays(dateStr, n) {
  const d = new Date(dateStr + 'T12:00:00');
  d.setDate(d.getDate() + n);
  return d.toISOString().slice(0, 10);
}

function formatDateLabel(dateStr) {
  const today = todayDate();
  if (dateStr === today) return 'Today';
  if (dateStr === addDays(today, -1)) return 'Yesterday';
  const d = new Date(dateStr + 'T12:00:00');
  const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${days[d.getDay()]}, ${months[d.getMonth()]} ${d.getDate()}`;
}

function getMonthName(month) {
  const months = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
  return months[month - 1] || '';
}

function getHistoryDate() {
  return store.getState().historyDate || todayDate();
}

function esc(s) {
  if (s == null) return '';
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

function escAttr(s) {
  if (s == null) return '';
  return String(s).replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// ── Project usage tracking (for "Suggested" group in the New Task picker) ──

const PROJECT_USAGE_KEY = 'pointeuse.projectUsage';
const SUGGESTED_LIMIT = 5;

function getProjectUsage() {
  try {
    return JSON.parse(localStorage.getItem(PROJECT_USAGE_KEY) || '{}') || {};
  } catch { return {}; }
}

function bumpProjectUsage(id) {
  if (!id) return;
  const usage = getProjectUsage();
  const entry = usage[id] || { count: 0, lastUsed: 0 };
  entry.count = (entry.count || 0) + 1;
  entry.lastUsed = Date.now();
  usage[id] = entry;
  try { localStorage.setItem(PROJECT_USAGE_KEY, JSON.stringify(usage)); } catch {}
}

/**
 * Populate a <select> with sorted projects, surfacing the user's most-used
 * projects as a "Suggested" optgroup on top and the rest alphabetized below.
 */
function populateProjectSelect(sel, projects) {
  if (!sel) return;
  const usage = getProjectUsage();
  const sorted = [...projects].sort((a, b) => a.name.localeCompare(b.name));

  const withUsage = sorted.filter(p => (usage[p.id]?.count || 0) > 0);
  withUsage.sort((a, b) => {
    const ua = usage[a.id], ub = usage[b.id];
    if (ub.count !== ua.count) return ub.count - ua.count;
    return (ub.lastUsed || 0) - (ua.lastUsed || 0);
  });
  const suggested = withUsage.slice(0, SUGGESTED_LIMIT);
  const suggestedIds = new Set(suggested.map(p => p.id));

  const optFor = p => `<option value="${p.id}">${esc(p.name)}</option>`;
  const suggestedGroup = suggested.length
    ? `<optgroup label="Suggested">${suggested.map(optFor).join('')}</optgroup>`
    : '';
  const allGroup = `<optgroup label="All projects">${sorted.filter(p => !suggestedIds.has(p.id)).map(optFor).join('')}</optgroup>`;

  sel.innerHTML = '<option value="">Select a project...</option>' + suggestedGroup + allGroup;
}

// ── Navigation ────────────────────────────────────────────────────────

function navigateTo(view) {
  store.setState({ view });
}

store.subscribe((state) => {
  $$('[data-view]').forEach(el => {
    el.classList.toggle('active', el.dataset.view === state.view);
  });
  $$('.nav-btn[data-nav]').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.nav === state.view);
  });
});

document.addEventListener('click', (e) => {
  const navBtn = e.target.closest('.nav-btn[data-nav]');
  if (navBtn) navigateTo(navBtn.dataset.nav);
});

// ── Toast ─────────────────────────────────────────────────────────────

let toastTimeout;
function showToast(msg, type = 'error') {
  const el = $('#toast');
  if (!el) return;
  el.textContent = msg;
  el.className = `toast visible toast-${type}`;
  clearTimeout(toastTimeout);
  toastTimeout = setTimeout(() => el.classList.remove('visible'), 4000);
}

// ── Manual entry composer ────────────────────────────────────────────

/** Queued rows for a date. get_entries_for_date never consults the queue, so
 *  this is a second, additive request rendered as visually distinct rows. */
async function refreshPendingFor(dateStr) {
  try {
    const pending = await api.getPendingForDate(dateStr);
    const rows = (pending || []).map(pendingToRow);
    return reconcileEntries(store.getState().todayEntries || [], rows);
  } catch (_) { return []; }
}

const composer = createEntryComposer({
  invoke,
  showToast,
  variant: 'tray',
  getViewedDate: () => getHistoryDate(),
  onChanged: async (date, info) => {
    if (info && (info.outcome === 'created' || info.outcome === 'restored') && info.entry) {
      store.setState({ justAddedKey: `${info.entry.task_id || 0}|${(info.entry.hours || 0).toFixed(2)}` });
    }
    // Route through the existing refresh so state.todayEntries changes
    // identity — #timer-breakdown, the header total and the weekly chart then
    // update for free via the subscribers that already exist.
    if (date === getHistoryDate()) {
      if (date === todayDate()) await refreshTodayEntries();
      else await goToHistoryDate(date);
    }
    store.setState({ todayPending: await refreshPendingFor(getHistoryDate()) });
  },
});

// ── Titlebar ──────────────────────────────────────────────────────────

document.querySelector('.titlebar')?.addEventListener('mousedown', async (e) => {
  if (e.target.closest('.titlebar-btn') || e.target.closest('.attendance-indicator')) return;
  if (e.buttons === 1) {
    try { await window.__TAURI__.window.getCurrentWindow().startDragging(); } catch (_) {}
  }
});

$('#titlebar-minimize')?.addEventListener('click', async () => {
  try { await window.__TAURI__.window.getCurrentWindow().minimize(); } catch (_) {}
});
$('#titlebar-close')?.addEventListener('click', async () => {
  try { await window.__TAURI__.window.getCurrentWindow().hide(); } catch (_) {}
});

// ── Login View ────────────────────────────────────────────────────────

$('#login-form')?.addEventListener('submit', async (e) => {
  e.preventDefault();
  const url = $('#login-url').value.trim();
  const database = $('#login-database').value.trim();
  const username = $('#login-username').value.trim();
  const password = $('#login-password').value;
  const errorEl = $('#login-error');
  const submitBtn = $('#login-submit');

  errorEl.textContent = '';
  submitBtn.disabled = true;
  submitBtn.textContent = 'Connecting...';

  try {
    const result = await api.login(url, database, username, password);
    store.setState({ auth: { authenticated: true, username: result.username, url } });
    navigateTo('timer');
    refreshAll();
  } catch (err) {
    errorEl.textContent = typeof err === 'string' ? err : err.message || 'Connection failed';
  } finally {
    submitBtn.disabled = false;
    submitBtn.textContent = 'Connect';
  }
});

// ── Timer View ────────────────────────────────────────────────────────

const timerDisplay = $('#timer-display');
const timerTaskName = $('#timer-task-name');
const timerProjectName = $('#timer-project-name');
const timerRing = $('#timer-ring');
const btnOpenOdoo = $('#btn-open-odoo');
const btnSelectTask = $('#btn-select-task');
const btnStopLog = $('#btn-stop-log');
const btnSwitchTask = $('#btn-switch-task');
const btnDiscard = $('#btn-discard');
const logForm = $('#log-form');
const timerStateWrap = $('#timer-state-wrap');
const timerStageSelect = $('#timer-stage-select');
let cachedStages = []; // available stages for the current task's project
let currentKanbanState = null;
let lastStageTaskId = null; // track which task we loaded stages for

// Odoo URL cache for "Open in Odoo" button
let cachedOdooUrl = '';
async function getOdooUrl() {
  if (cachedOdooUrl) return cachedOdooUrl;
  try {
    const conn = await api.getSavedConnection();
    cachedOdooUrl = (conn?.url || '').replace(/\/$/, '');
    return cachedOdooUrl;
  } catch (_) { return ''; }
}

async function openTaskInOdoo(taskId) {
  const url = await getOdooUrl();
  if (!url) { showToast('Odoo URL not available'); return; }
  const taskUrl = `${url}/web#id=${taskId}&model=project.task&view_type=form`;
  try {
    if (window.__TAURI__?.opener?.openUrl) {
      await window.__TAURI__.opener.openUrl(taskUrl);
    } else {
      window.open(taskUrl, '_blank');
    }
  } catch (_) {
    window.open(taskUrl, '_blank');
  }
}

btnOpenOdoo?.addEventListener('click', () => {
  const { timer } = store.getState();
  if (timer.task_id) openTaskInOdoo(timer.task_id);
});

store.subscribe((state) => {
  const t = state.timer;

  // Timer ring animation
  if (timerRing) {
    timerRing.classList.toggle('running', t.is_running);
  }

  // Update titlebar with timer status
  const titleEl = document.querySelector('.titlebar-title');
  if (titleEl) titleEl.textContent = t.is_running ? `${formatTime(t.elapsed_secs)} — ${t.task_name || ''}` : 'Pointeuse';

  // "Started on <device>" — only when this run came from somewhere else
  const originBadge = document.getElementById('timer-origin-badge');
  if (originBadge) {
    const elsewhere = t.is_running
      && t.origin_device
      && state.device.id
      && t.origin_device !== state.device.id;
    originBadge.style.display = elsewhere ? '' : 'none';
    if (elsewhere) originBadge.textContent = `started on ${t.origin_label || 'another device'}`;
  }

  if (t.is_running) {
    timerDisplay.textContent = formatTime(t.elapsed_secs);
    timerTaskName.textContent = t.task_name || 'Unknown task';
    timerProjectName.textContent = t.project_name || '';
    btnSelectTask.style.display = 'none';
    btnStopLog.style.display = '';
    btnSwitchTask.style.display = '';
    btnDiscard.style.display = '';
    if (btnOpenOdoo) btnOpenOdoo.style.display = '';
    if (timerStateWrap) timerStateWrap.style.display = '';
    // Load stages if task changed
    if (t.task_id && t.task_id !== lastStageTaskId) {
      lastStageTaskId = t.task_id;
      loadTaskStages(t.task_id, t.project_id || 0);
    }
  } else if (state.stoppedTimer) {
    timerDisplay.textContent = formatTime(state.stoppedTimer.elapsed_secs);
    btnSelectTask.style.display = 'none';
    btnStopLog.style.display = 'none';
    btnSwitchTask.style.display = 'none';
    btnDiscard.style.display = 'none';
    if (btnOpenOdoo) btnOpenOdoo.style.display = 'none';
    if (timerStateWrap) timerStateWrap.style.display = 'none';
  } else {
    timerDisplay.textContent = '00:00:00';
    timerTaskName.textContent = 'No task selected';
    timerProjectName.textContent = '';
    btnSelectTask.style.display = '';
    btnStopLog.style.display = 'none';
    btnSwitchTask.style.display = 'none';
    btnDiscard.style.display = 'none';
    if (btnOpenOdoo) btnOpenOdoo.style.display = 'none';
    if (timerStateWrap) timerStateWrap.style.display = 'none';
    lastStageTaskId = null;
  }

  // Log form
  if (state.stoppedTimer) {
    logForm.style.display = '';
    $('#log-form-task').textContent = state.stoppedTimer.task_name;
    $('#log-form-time').textContent = formatTime(state.stoppedTimer.elapsed_secs);
    // Show stage selector in log form if stages are cached
    const logStageWrap = $('#log-stage-wrap');
    if (logStageWrap) logStageWrap.style.display = cachedStages.length > 0 ? '' : 'none';
  } else {
    logForm.style.display = 'none';
  }

  // Today total
  const totalHours = (state.todayEntries || []).reduce((sum, e) => sum + (e.hours || 0), 0);
  const todayEl = $('#today-total');
  if (todayEl) todayEl.textContent = formatHours(totalHours);
});

btnSelectTask?.addEventListener('click', () => showTaskPrompt('checkin'));

btnSwitchTask?.addEventListener('click', () => showTaskPrompt('switch'));

// ── Task Stage Management ─────────────────────────────────────────────

async function loadTaskStages(taskId, projectId) {
  if (!timerStageSelect) return;
  timerStageSelect.innerHTML = '<option value="">Loading...</option>';
  try {
    const info = await api.getTaskStages(taskId, projectId);
    cachedStages = info.available_stages || [];
    const currentId = info.stage_id;
    currentKanbanState = info.kanban_state || 'normal';
    timerStageSelect.innerHTML = cachedStages.map(s =>
      `<option value="${s.id}"${s.id === currentId ? ' selected' : ''}>${esc(s.name)}</option>`
    ).join('');
    // Update kanban state dots
    updateKanbanDots('#kanban-state-btns', currentKanbanState);
    // Update state selects
    const timerStateSelect = $('#timer-state-select');
    if (timerStateSelect && info.state) timerStateSelect.value = info.state;
    // Also populate log form stage selector
    const logSelect = $('#log-stage-select');
    if (logSelect) {
      logSelect.innerHTML = '<option value="">Don\'t change</option>' +
        cachedStages.map(s =>
          `<option value="${s.id}"${s.id === currentId ? ' selected' : ''}>${esc(s.name)}</option>`
        ).join('');
    }
    updateKanbanDots('#log-kanban-btns', currentKanbanState);
    const logStateSelect = $('#log-state-select');
    if (logStateSelect && info.state) logStateSelect.value = info.state;
  } catch (_) {
    timerStageSelect.innerHTML = '<option value="">Unavailable</option>';
  }
}

function updateKanbanDots(containerSel, activeState) {
  const container = $(containerSel);
  if (!container) return;
  container.querySelectorAll('.kanban-dot').forEach(dot => {
    dot.classList.toggle('active', dot.dataset.state === activeState);
  });
}

// Change stage from timer view dropdown
timerStageSelect?.addEventListener('change', async (e) => {
  const stageId = parseInt(e.target.value);
  const t = store.getState().timer;
  if (!stageId || !t.task_id) return;
  try {
    await api.updateTaskStage(t.task_id, stageId);
    const stageName = cachedStages.find(s => s.id === stageId)?.name || '';
    showToast(`Stage: "${stageName}"`, 'success');
  } catch (err) { showToast(String(err)); }
});

// State select change (timer view - immediate update)
$('#timer-state-select')?.addEventListener('change', async (e) => {
  const newState = e.target.value;
  const t = store.getState().timer;
  if (!newState || !t.task_id) return;
  try {
    await api.updateTaskState(t.task_id, newState);
    const label = e.target.options[e.target.selectedIndex]?.text || newState;
    showToast(`State: ${label}`, 'success');
  } catch (err) { showToast(String(err)); }
});

// Kanban state dot clicks (timer view + log form)
document.addEventListener('click', async (e) => {
  const dot = e.target.closest('.kanban-dot[data-state]');
  if (!dot) return;
  const newState = dot.dataset.state;
  const t = store.getState().timer;
  const taskId = t.task_id || store.getState().stoppedTimer?.task_id;
  if (!taskId) return;

  // If in log form, just update the visual (actual update happens on submit)
  const isLogForm = dot.closest('#log-kanban-btns');
  if (isLogForm) {
    currentKanbanState = newState;
    updateKanbanDots('#log-kanban-btns', newState);
    return;
  }

  // Timer view: update immediately
  try {
    await api.updateTaskKanbanState(taskId, newState);
    currentKanbanState = newState;
    updateKanbanDots('#kanban-state-btns', newState);
    const labels = { normal: 'In Progress', done: 'Ready', blocked: 'Blocked' };
    showToast(`Status: ${labels[newState] || newState}`, 'success');
  } catch (err) { showToast(String(err)); }
});

btnStopLog?.addEventListener('click', async () => {
  try {
    const result = await api.stopTimer();
    store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: result });
    $('#log-description').value = '';
    $('#log-description').focus();
  } catch (err) { showToast(String(err)); }
});

btnDiscard?.addEventListener('click', async () => {
  if (!btnDiscard.dataset.confirmed) {
    btnDiscard.dataset.confirmed = 'true';
    btnDiscard.textContent = 'Sure?';
    setTimeout(() => {
      if (btnDiscard.dataset.confirmed) {
        delete btnDiscard.dataset.confirmed;
        btnDiscard.textContent = 'Discard';
      }
    }, 3000);
    return;
  }
  delete btnDiscard.dataset.confirmed;
  btnDiscard.textContent = 'Discard';
  try {
    await api.discardTimer();
    store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: null });
  } catch (err) { showToast(String(err)); }
});

$('#btn-log-submit')?.addEventListener('click', async () => {
  const st = store.getState().stoppedTimer;
  if (!st) return;
  const desc = $('#log-description').value.trim() || st.task_name;
  const hours = st.elapsed_secs / 3600;
  try {
    await api.logTime(st.task_id, st.project_id || 0, st.task_name, st.project_name, desc, hours, todayDate());
    // Update stage if user selected a different one
    const logStageVal = parseInt($('#log-stage-select')?.value);
    if (logStageVal && st.task_id) {
      try { await api.updateTaskStage(st.task_id, logStageVal); } catch (_) {}
    }
    // Update kanban state if changed
    if (currentKanbanState && st.task_id) {
      try { await api.updateTaskKanbanState(st.task_id, currentKanbanState); } catch (_) {}
    }
    // Update task state if user changed it
    const logStateVal = $('#log-state-select')?.value;
    if (logStateVal && st.task_id) {
      try { await api.updateTaskState(st.task_id, logStateVal); } catch (_) {}
    }
    store.setState({ stoppedTimer: null });
    showToast('Time logged!', 'success');
    refreshTodayEntries();
  } catch (err) { showToast(String(err)); }
});

$('#btn-log-cancel')?.addEventListener('click', () => {
  store.setState({ stoppedTimer: null });
});

// ── Attendance ────────────────────────────────────────────────────────

store.subscribe((state) => {
  const att = state.attendance;
  const dot = $('#attendance-dot');
  const text = $('#attendance-text');
  const toggleBtn = $('#btn-attendance-toggle');
  const toggleText = $('#attendance-toggle-text');
  const toggleDot = toggleBtn?.querySelector('.att-dot');

  if (dot) dot.className = `attendance-dot ${att.is_checked_in ? 'checked-in' : 'checked-out'}`;
  if (text) text.textContent = att.is_checked_in ? 'In' : '';

  if (toggleText) toggleText.textContent = att.is_checked_in ? 'Check Out' : 'Check In';
  if (toggleBtn) toggleBtn.classList.toggle('checked-in', att.is_checked_in);
  if (toggleDot) toggleDot.className = `att-dot ${att.is_checked_in ? 'checked-in' : ''}`;
});

$('#btn-attendance-toggle')?.addEventListener('click', async () => {
  const att = store.getState().attendance;
  try {
    if (att.is_checked_in) {
      // Auto-stop and log timer if running
      const timer = store.getState().timer;
      if (timer.is_running) {
        try {
          const stopped = await api.stopTimer();
          const hours = stopped.elapsed_secs / 3600;
          await api.logTime(stopped.task_id, stopped.project_id || 0, stopped.task_name, stopped.project_name, stopped.task_name, hours, todayDate());
          store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: null });
          showToast(`Auto-logged ${formatTime(stopped.elapsed_secs)} for ${stopped.task_name}`, 'success');
        } catch (timerErr) {
          showToast('Warning: could not auto-log timer. Time may be lost.', 'error');
          console.warn('Auto-stop timer on checkout failed:', timerErr);
        }
      }
      const result = await api.attendanceCheckOut();
      store.setState({ attendance: { is_checked_in: false, attendance_id: null, check_in_time: null } });
      showToast(`Checked out. Today: ${formatHours(result.worked_hours_today)}`, 'success');
    } else {
      const result = await api.attendanceCheckIn();
      store.setState({ attendance: { is_checked_in: true, attendance_id: result.attendance_id, check_in_time: result.check_in_time } });
      showToast('Checked in!', 'success');
      // Show task selection prompt after check-in
      showTaskPrompt();
    }
  } catch (err) { showToast(String(err)); }
});

// ── Idle Reminder ─────────────────────────────────────────────────────

const reminderPopup = $('#reminder-popup');

// Listen for task picker request from reminder popup window
try {
  window.__TAURI__?.event?.listen('open_task_picker', (event) => {
    const mode = event.payload?.mode || 'switch';
    showTaskPrompt(mode);
  });
} catch (_) {}

// Listen for attendance changes from background polling
try {
  window.__TAURI__?.event?.listen('attendance_changed', (event) => {
    const status = event.payload;
    store.setState({ attendance: status });
  });
} catch (_) {}

// Listen for tray auto-stop event
try {
  window.__TAURI__?.event?.listen('timer_auto_stopped', (event) => {
    store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: null });
    reminderPopup.classList.remove('visible');
    showToast(`Timer auto-logged on check-out`, 'success');
    refreshTodayEntries();
  });
} catch (_) {}

// Cross-device sync: another instance of Pointeuse on the same Odoo login
// started or stopped a timer, and the backend has already mirrored it here.
try {
  window.__TAURI__?.event?.listen('timer_remote_started', async (event) => {
    const p = event.payload || {};
    await refreshTimerState();
    reminderPopup.classList.remove('visible');
    const where = p.origin_label ? ` on ${p.origin_label}` : '';
    showToast(`Picked up "${p.task_name}" started${where}`, 'success');
  });

  window.__TAURI__?.event?.listen('timer_remote_stopped', async (event) => {
    const p = event.payload || {};
    store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: null });
    await refreshTimerState();
    reminderPopup.classList.remove('visible');
    showToast(`"${p.task_name}" was stopped on another device`, 'success');
    refreshTodayEntries();
  });

  // A live session was settled in Odoo — today's totals moved.
  window.__TAURI__?.event?.listen('timesheet_changed', () => {
    refreshTodayEntries();
  });
} catch (_) {}

// Reconcile as soon as the window is looked at again: waiting out the poll
// interval would show a stale timer at exactly the wrong moment.
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && store.getState().auth.authenticated) {
    api.syncDevicesNow().catch(() => {});
    refreshTimerState();
  }
});

// Listen for explicit reminder dismiss from backend (e.g., external checkout)
try {
  window.__TAURI__?.event?.listen('dismiss_reminder', () => {
    reminderPopup.classList.remove('visible');
  });
  // Refresh today entries when reminder popup logs time (stop or quick-switch)
  window.__TAURI__?.event?.listen('reminder_timer_logged', () => {
    refreshTodayEntries();
    renderTimerBreakdown();
  });
  // Notify user when time is redirected from private task to default task
  window.__TAURI__?.event?.listen('time_redirected', (event) => {
    const d = event.payload;
    showToast(`Private task "${d.original_task}" — time logged to "${d.default_task}" instead`, 'success');
  });
} catch (_) {}

// Listen for Tauri event from backend
try {
  window.__TAURI__?.event?.listen('show_idle_reminder', async (event) => {
    const payload = event.payload;
    $('#reminder-title').textContent = `Still working on ${payload.task_name}...`;
    $('#reminder-message').textContent = `Timer running: ${payload.task_name} (${formatTime(payload.elapsed_secs)})`;
    // Build quick-switch buttons for in-app overlay
    const qsContainer = document.getElementById('reminder-quickswitch');
    if (qsContainer) {
      const qs = payload.quick_switch || [];
      const currentTaskId = payload.task_id;
      const filtered = qs.filter(q => q.task_id !== currentTaskId);
      if (filtered.length > 0) {
        const mainItems = filtered.filter(q => q.slot === 'main');
        const smallItems = filtered.filter(q => q.slot === 'small');
        let html = '<div class="reminder-qs-header"><span class="reminder-qs-label">Quick Switch</span><button class="btn btn-secondary btn-xs" id="reminder-all-tasks">All Tasks &#8250;</button></div>';
        if (mainItems.length > 0) {
          html += '<div class="reminder-qs-main">';
          for (const q of mainItems) {
            html += `<button class="reminder-qs-card reminder-qs-btn" data-qs-task-id="${q.task_id}" data-qs-task-name="${escAttr(q.task_name)}" data-qs-project-id="${q.project_id}" data-qs-project-name="${escAttr(q.project_name)}"><span class="reminder-qs-card-name">${esc(q.task_name)}</span><span class="reminder-qs-card-proj">${esc(q.project_name)}</span></button>`;
          }
          html += '</div>';
        }
        if (smallItems.length > 0) {
          html += '<div class="reminder-qs-small">';
          for (const q of smallItems) {
            html += `<button class="reminder-qs-pill reminder-qs-btn" data-qs-task-id="${q.task_id}" data-qs-task-name="${escAttr(q.task_name)}" data-qs-project-id="${q.project_id}" data-qs-project-name="${escAttr(q.project_name)}">${esc(q.task_name)}</button>`;
          }
          html += '</div>';
        }
        qsContainer.innerHTML = html;
        qsContainer.style.display = '';
        // Bind "All Tasks" button
        document.getElementById('reminder-all-tasks')?.addEventListener('click', () => {
          reminderPopup.classList.remove('visible');
          try { api.dismissIdleReminder(); } catch (_) {}
          showTaskPrompt('switch');
        });
      } else {
        qsContainer.innerHTML = '';
        qsContainer.style.display = 'none';
      }
    }
    reminderPopup.classList.add('visible');
  });
} catch (_) {}

// ── Notification action handler (mobile) ────────────────────────────
// When the user taps a notification or its action button, the notification
// plugin fires an "actionPerformed" event. We use this to handle:
//   - Tapping the ongoing timer notification → show the app (already foregrounded)
//   - Tapping the reminder notification → open the task picker for quick-switch
try {
  window.__TAURI__?.core?.addPluginListener?.('notification', 'actionPerformed', async (event) => {
    const actionId = event?.actionId || 'tap';
    const extra = event?.notification?.extra || {};
    const notifId = event?.notification?.id;

    console.log('Notification action:', actionId, 'notifId:', notifId, 'extra:', JSON.stringify(extra));

    // Reminder notification actions (match by action ID or notification ID)
    if (actionId === 'stop' || actionId === 'change_task' || actionId === 'continue' ||
        notifId === 9002 || extra?.type === 'reminder') {
      if (actionId === 'stop') {
        // Stop timer, log time, dismiss
        console.log('stop: stopping timer');
        try {
          await api.dismissIdleReminder();
          const result = await api.stopTimer();
          if (result && result.elapsed_secs > 0) {
            const h = result.elapsed_secs / 3600;
            const d = new Date().toISOString().slice(0, 10);
            await api.logTime(result.task_id, result.project_id || 0, result.task_name, result.project_name || '', result.task_name, h, d);
          }
        } catch (_) {}
        reminderPopup?.classList?.remove('visible');
      } else if (actionId === 'change_task') {
        // Dismiss and open task picker (small delay for app to fully foreground)
        try { await api.dismissIdleReminder(); } catch (_) {}
        reminderPopup?.classList?.remove('visible');
        console.log('change_task: opening task picker in 300ms');
        setTimeout(() => {
          console.log('change_task: calling showTaskPrompt now');
          showTaskPrompt('switch');
        }, 300);
      } else {
        // "continue" or "tap" → just dismiss the reminder
        try { await api.dismissIdleReminder(); } catch (_) {}
        reminderPopup?.classList?.remove('visible');
      }
      return;
    }
    // Ongoing timer notification tapped (id 9001) → just bring app to front (already done by Android)
  });
} catch (_) {}

// ── Global sync progress bar ────────────────────────────────────────
let _syncCounter = 0;
function _ensureSyncBar() {
  let bar = document.getElementById('global-sync-bar');
  if (!bar) {
    bar = document.createElement('div');
    bar.id = 'global-sync-bar';
    bar.className = 'global-sync-bar';
    document.body.prepend(bar);
  }
  return bar;
}
function _showGlobalSync() {
  _syncCounter++;
  _ensureSyncBar().classList.add('active');
}
function _hideGlobalSync() {
  _syncCounter = Math.max(0, _syncCounter - 1);
  if (_syncCounter === 0) {
    _ensureSyncBar().classList.remove('active');
  }
}
try {
  window.__TAURI__?.event?.listen('cache_sync_start', () => _showGlobalSync());
  window.__TAURI__?.event?.listen('cache_sync_done', () => _hideGlobalSync());
  // Listen for tasks_refreshed from background refresh
  window.__TAURI__?.event?.listen('tasks_refreshed', (event) => {
    const tasks = event.payload;
    if (Array.isArray(tasks)) {
      store.setState({ myTasks: tasks });
    }
  });
} catch (_) {}

// Listen for cache refresh events (background sync updates)
try {
  window.__TAURI__?.event?.listen('entries_refreshed', (event) => {
    const { date, entries } = event.payload;
    // Only update if we're currently viewing this date
    const currentDate = getHistoryDate();
    if (date === currentDate || date === todayDate()) {
      const rows = entries || [];
      // Authoritative reconciliation: drop any queued row now represented
      // upstream, otherwise the day double-counts for one refresh cycle.
      store.setState({
        todayEntries: rows,
        todayPending: reconcileEntries(rows, store.getState().todayPending || []),
      });
    }
    hideSyncIndicator();
  });

  // Fires synchronously at the end of every mutation, one network round-trip
  // BEFORE entries_refreshed, so the other window reacts immediately.
  window.__TAURI__?.event?.listen('ledger_changed', async (event) => {
    const p = event.payload || {};
    const currentDate = getHistoryDate();
    if (p.date && p.date !== currentDate) return;
    store.setState({ todayPending: await refreshPendingFor(currentDate) });
    try {
      const ss = await api.getSyncStatus();
      store.setState({ syncStatus: ss });
    } catch (_) {}
  });
  window.__TAURI__?.event?.listen('monthly_refreshed', (event) => {
    const summary = event.payload;
    const { historyMode, monthYear } = store.getState();
    if (historyMode === 'month' && monthYear &&
        monthYear.year === summary.year && monthYear.month === summary.month) {
      renderMonthSummary(summary);
    }
    hideSyncIndicator();
  });
  window.__TAURI__?.event?.listen('analysis_refreshed', (event) => {
    const analysis = event.payload;
    const currentDate = getHistoryDate();
    if (analysis.date === currentDate) {
      renderAnalysis(analysis);
    }
    hideSyncIndicator();
  });

  // Sync events: duplicates found, entries rejected
  window.__TAURI__?.event?.listen('sync_duplicate_found', (event) => {
    const d = event.payload;
    showToast(`Duplicate detected for ${d.date} (${d.hours.toFixed(2)}h) — review in sync panel`, 'warning');
  });
  window.__TAURI__?.event?.listen('sync_entry_rejected', (event) => {
    const d = event.payload;
    showToast(`Sync rejected: ${d.error?.substring(0, 60)}`, 'error');
  });
} catch (_) {}

$('#reminder-action')?.addEventListener('click', async () => {
  // "Keep Going" — dismiss and continue
  reminderPopup.classList.remove('visible');
  try { await api.dismissIdleReminder(); } catch (_) {}
});

$('#reminder-dismiss')?.addEventListener('click', async () => {
  // "Dismiss" — stop timer
  reminderPopup.classList.remove('visible');
  try {
    await api.dismissIdleReminder();
    const result = await api.stopTimer();
    store.setState({ timer: { is_running: false, elapsed_secs: 0 }, stoppedTimer: result });
    navigateTo('timer');
  } catch (_) {}
});

// Quick-switch from in-app reminder overlay
document.addEventListener('click', async (e) => {
  const btn = e.target.closest('.reminder-qs-btn');
  if (!btn) return;
  const taskId = parseInt(btn.dataset.qsTaskId);
  const taskName = btn.dataset.qsTaskName;
  const projectId = parseInt(btn.dataset.qsProjectId) || 0;
  const projectName = btn.dataset.qsProjectName;
  reminderPopup.classList.remove('visible');
  try {
    await api.dismissIdleReminder();
    const timer = store.getState().timer;
    if (timer.is_running) {
      const stopped = await api.stopTimer();
      const hours = stopped.elapsed_secs / 3600;
      await api.logTime(stopped.task_id, stopped.project_id || 0, stopped.task_name, stopped.project_name, stopped.task_name, hours, todayDate());
      showToast(`Logged ${formatTime(stopped.elapsed_secs)} for ${stopped.task_name}`, 'success');
    }
    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    navigateTo('timer');
  } catch (err) { showToast(String(err)); }
});

// ── Tasks View ────────────────────────────────────────────────────────

let searchTimeout;
let activeTab = 'recent';

$$('.tab-bar .tab').forEach(tab => {
  tab.addEventListener('click', () => {
    $$('.tab-bar .tab').forEach(t => t.classList.remove('active'));
    tab.classList.add('active');
    activeTab = tab.dataset.tab;
    loadTasksForTab();
  });
});

$('#task-search')?.addEventListener('input', (e) => {
  clearTimeout(searchTimeout);
  const q = e.target.value.trim();
  if (q.length > 0) {
    $$('.tab-bar .tab').forEach(t => t.classList.toggle('active', t.dataset.tab === 'search'));
    activeTab = 'search';
    searchTimeout = setTimeout(() => searchTasks(q), 300);
  } else {
    $$('.tab-bar .tab').forEach(t => t.classList.toggle('active', t.dataset.tab === 'recent'));
    activeTab = 'recent';
    loadTasksForTab();
  }
});

async function loadTasksForTab() {
  const listEl = $('#task-list');
  try {
    let tasks = [];
    if (activeTab === 'recent') tasks = await api.getRecentTasks();
    else if (activeTab === 'my-tasks') tasks = await api.getMyTasks();
    renderTaskList(listEl, tasks);
  } catch (err) { showToast(String(err)); }
}

async function searchTasks(query) {
  try {
    const tasks = await api.searchTasks(query, null);
    renderTaskList($('#task-list'), tasks, { forceExpand: true });
  } catch (err) { showToast(String(err)); }
}

// Track which project groups are collapsed (session only)
const collapsedGroups = new Set();

function renderTaskList(container, tasks, { grouped = true, forceExpand = false, suggested = false, otherTasks = null } = {}) {
  // Apply done-task filter
  const hideDone = store.getState().hideDoneTasks;
  const filterDone = (arr) => (arr || []).filter(t => {
    const st = (t.state || '').toLowerCase();
    return st !== '1_done' && st !== '1_canceled';
  });
  if (hideDone) {
    tasks = filterDone(tasks);
    if (otherTasks) otherTasks = filterDone(otherTasks);
  }

  if ((!tasks || tasks.length === 0) && (!otherTasks || otherTasks.length === 0)) {
    container.innerHTML = '<div class="empty-state"><p>No tasks found</p></div>';
    return;
  }

  // If we have suggested + other, render suggested as flat list, other as grouped
  if (suggested && otherTasks) {
    let html = '';
    if (tasks && tasks.length > 0) {
      html += '<div class="task-section-header"><span class="task-section-label">Suggested</span></div>';
      html += tasks.map(t => renderTaskItem(t)).join('');
    }
    if (otherTasks && otherTasks.length > 0) {
      html += '<div class="task-section-header task-section-other"><span class="task-section-label">All Tasks</span></div>';
      html += renderGroupedTasks(otherTasks, forceExpand);
    }
    container.innerHTML = html;
    return;
  }

  if (grouped) {
    container.innerHTML = renderGroupedTasks(tasks, forceExpand);
  } else {
    container.innerHTML = tasks.map(t => renderTaskItem(t)).join('');
  }
}

function renderGroupedTasks(tasks, forceExpand = false) {
  const groups = {};
  for (const t of tasks) {
    const proj = t.project_name || 'No project';
    if (!groups[proj]) groups[proj] = [];
    groups[proj].push(t);
  }
  const sortedKeys = Object.keys(groups).sort((a, b) => {
    if (a === 'No project') return 1;
    if (b === 'No project') return -1;
    return a.localeCompare(b);
  });
  let html = '';
  for (const proj of sortedKeys) {
    const isCollapsed = !forceExpand && collapsedGroups.has(proj);
    const count = groups[proj].length;
    html += `<div class="task-group${isCollapsed ? ' collapsed' : ''}" data-project="${escAttr(proj)}">`;
    html += `<div class="task-group-header">
      <svg class="task-group-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
      <span class="task-group-name">${esc(proj)}</span>
      <span class="task-group-count">${count}</span>
    </div>`;
    html += `<div class="task-group-body">`;
    for (const t of groups[proj]) {
      html += renderTaskItem(t);
    }
    html += `</div></div>`;
  }
  return html;
}

// Delegate click on group headers to toggle collapse
document.addEventListener('click', (e) => {
  const header = e.target.closest('.task-group-header');
  if (!header) return;
  const group = header.closest('.task-group');
  if (!group) return;
  const proj = group.dataset.project;
  group.classList.toggle('collapsed');
  if (group.classList.contains('collapsed')) {
    collapsedGroups.add(proj);
  } else {
    collapsedGroups.delete(proj);
  }
});

function renderTaskItem(t) {
  const reason = t.reason ? `<span class="task-item-reason">${esc(t.reason)}</span>` : '';
  return `
    <div class="task-item" data-task-id="${t.id}" data-task-name="${escAttr(t.name)}"
         data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
      <div class="task-item-info">
        <span class="task-item-name">${esc(t.name)}</span>
        <span class="task-item-project">${esc(t.project_name || 'No project')}${t.stage_name ? ' · ' + esc(t.stage_name) : ''}${reason ? ' · ' + reason : ''}</span>
      </div>
      <button class="task-item-odoo-link" data-task-id="${t.id}" title="Open in Odoo">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
      </button>
      <svg class="task-item-arrow" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
    </div>
  `;
}

// Open in Odoo click on task list items (delegated)
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.task-item-odoo-link');
  if (!btn) return;
  e.stopPropagation();
  e.preventDefault();
  const taskId = parseInt(btn.dataset.taskId);
  if (taskId) openTaskInOdoo(taskId);
});

// Task click → start timer (main task list only, not picker popup)
document.addEventListener('click', async (e) => {
  const item = e.target.closest('.task-item[data-task-id]');
  if (!item) return;
  // Skip if inside the task picker popup (handled separately)
  if (item.closest('#task-picker-list')) return;
  const taskId = parseInt(item.dataset.taskId);
  const taskName = item.dataset.taskName;
  const projectId = parseInt(item.dataset.projectId) || 0;
  const projectName = item.dataset.projectName;
  try {
    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    navigateTo('timer');
  } catch (err) { showToast(String(err)); }
});

$('#btn-new-task')?.addEventListener('click', async () => {
  navigateTo('task-create');
  try {
    const projects = await api.getProjects();
    populateProjectSelect($('#new-task-project'), projects);
  } catch (err) { showToast(String(err)); }
});

// ── Task Create View ──────────────────────────────────────────────────

$('#btn-back-from-create')?.addEventListener('click', () => navigateTo('tasks'));
$('#btn-cancel-create')?.addEventListener('click', () => navigateTo('tasks'));

$('#task-create-form')?.addEventListener('submit', async (e) => {
  e.preventDefault();
  const name = $('#new-task-name').value.trim();
  const projectId = parseInt($('#new-task-project').value);
  if (!name || !projectId) return;
  try {
    const task = await api.createTask(name, projectId);
    bumpProjectUsage(projectId);
    await api.startTimer(task.id, task.name, task.project_id || 0, task.project_name || '');
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    navigateTo('timer');
    showToast('Task created!', 'success');
  } catch (err) { showToast(String(err)); }
});

// ── History View ──────────────────────────────────────────────────────

store.subscribe((state) => {
  const entries = state.todayEntries || [];
  const listEl = $('#history-list');
  if (!listEl) return;

  // Update date navigation display
  const date = getHistoryDate();
  const dateLabel = $('#history-date-label');
  const dateSub = $('#history-date-sub');
  if (dateLabel) dateLabel.textContent = formatDateLabel(date);
  if (dateSub) dateSub.textContent = date === todayDate() ? '' : date;

  // Show/hide day vs month views
  const monthView = $('#month-view');
  const dayElems = [$('#history-list'), $('#analysis-panel'), $('.analysis-trigger')];
  if (state.historyMode === 'month') {
    dayElems.forEach(el => { if (el) el.style.display = 'none'; });
    if (monthView) monthView.style.display = '';
  } else {
    dayElems.forEach(el => { if (el) el.style.display = ''; });
    if (monthView) monthView.style.display = 'none';

    const pendingRows = state.todayPending || [];
    if (entries.length === 0 && pendingRows.length === 0) {
      listEl.innerHTML = `<div class="empty-state">
        <p>No time entries for ${formatDateLabel(date)}</p>
        <button class="btn btn-sm btn-primary" id="history-empty-add">Add entry</button>
      </div>`;
    } else {
      const totalH = entries.reduce((s, e) => s + (e.hours || 0), 0);
      let chartHtml = '<div class="timelog-day-chart">';
      for (const e of entries) {
        const pct = totalH > 0 ? Math.min(100, ((e.hours || 0) / totalH) * 100) : 0;
        chartHtml += `<div class="day-bar-row">
          <span class="day-bar-label" title="${escAttr(e.project_name || '')}">${esc(e.task_name)}</span>
          <div class="day-bar-track"><div class="day-bar-fill" style="width:${pct}%"></div></div>
          <span class="day-bar-value">${formatHours(e.hours || 0)}</span>
        </div>`;
      }
      chartHtml += '</div>';

      listEl.innerHTML = chartHtml
        + entries.map(e => renderHistoryRow(e, state.justAddedKey)).join('')
        + pendingRows.map(p => renderHistoryRow(p, null)).join('');
    }
    // Deferred: setState inside a subscriber would re-enter the whole
    // listener loop synchronously. The flash is one-shot either way.
    if (state.justAddedKey) setTimeout(() => store.setState({ justAddedKey: null }), 0);
  }

  // Queued hours are reported as their own figure, never folded into the
  // Odoo total — that is honest about what the employer can actually see,
  // and it sidesteps the double-count while a row is still in the queue.
  const total = entries.reduce((s, e) => s + (e.hours || 0), 0);
  const pendingH = (state.todayPending || []).reduce((s, e) => s + (e.hours || 0), 0);
  const totalEl = $('#history-total');
  if (totalEl) {
    totalEl.innerHTML = esc(formatHours(total))
      + (pendingH > 0 ? ` <span class="ec-queued-note">+${esc(formatHours(pendingH))} queued</span>` : '');
  }

  const badge = $('#sync-badge');
  if (badge) {
    const counts = state.syncStatus?.counts;
    const pending = (counts?.pending || 0) + (counts?.failed || 0);
    const review = state.syncStatus?.needs_review || 0;
    const total = pending + review;
    badge.textContent = review > 0 ? `${total}!` : total;
    badge.style.display = total > 0 ? '' : 'none';
    badge.title = review > 0
      ? `${pending} pending, ${review} need review (duplicates/errors)`
      : `${pending} pending`;
    badge.classList.toggle('review-needed', review > 0);
  }

  // Update analyze button text
  const analyzeBtn = $('#btn-analyze-day');
  if (analyzeBtn) {
    analyzeBtn.textContent = date === todayDate() ? 'Analyze Day' : `Analyze ${formatDateLabel(date)}`;
  }
});

/** One history row, with actions gated on `source` (see entryCapabilities). */
function renderHistoryRow(e, justAddedKey) {
  const cap = entryCapabilities(e);
  const key = `${e.task_id || 0}|${(e.hours || 0).toFixed(2)}`;
  const cls = [
    'history-item',
    cap.kind === 'pending' ? 'is-pending' : '',
    cap.kind === 'local' ? 'is-readonly' : '',
    justAddedKey && justAddedKey === key ? 'just-added' : '',
  ].filter(Boolean).join(' ');

  return `<div class="${cls}" ${rowAttrs(e)}>
    <div class="history-item-info">
      <span class="history-item-name">${esc(e.task_name)}${cap.kind === 'pending' ? pendingBadgeHtml(e) : ''}</span>
      <span class="history-item-desc">${esc(e.description || '')}</span>
      ${e.last_error ? `<span class="ec-pending-error">${esc(e.last_error)}</span>` : ''}
    </div>
    <span class="history-item-hours">${formatHours(e.hours || 0)}</span>
    ${rowActionsHtml(e)}
  </div>`;
}

/** Resolve a row element back to the entry object it was rendered from. */
function findHistoryRowEntry(rowEl) {
  const { todayEntries, todayPending } = store.getState();
  const pendingId = rowEl.dataset.pendingId;
  if (pendingId) return (todayPending || []).find(p => String(p.pending_id) === pendingId) || null;
  const entryId = rowEl.dataset.entryId;
  if (entryId) return (todayEntries || []).find(e => String(e.id) === entryId) || null;
  return null;
}

function openComposerForHistoryRow(entry, action) {
  const cap = entryCapabilities(entry);
  const seed = {
    date: entry.date || getHistoryDate(),
    taskId: entry.task_id,
    taskName: entry.task_name,
    projectId: entry.project_id || 0,
    projectName: entry.project_name || '',
    description: entry.description || '',
    durationHours: entry.hours,
  };
  if (action === 'duplicate') {
    composer.open({ ...seed, mode: 'create', allowDuplicate: true, description: '' });
  } else if (cap.kind === 'pending') {
    composer.open({ ...seed, mode: 'repair', pendingId: entry.pending_id });
  } else if (cap.kind === 'odoo') {
    composer.open({ ...seed, mode: 'edit', odooId: entry.id });
  }
}

// Row actions are delegated — #history-list is rebuilt on every setState.
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.history-item .ec-row-btn[data-ec-act]');
  if (!btn || btn.disabled) return;
  e.stopPropagation();
  const rowEl = btn.closest('.history-item');
  const entry = findHistoryRowEntry(rowEl);
  if (!entry) return;
  const act = btn.dataset.ecAct;
  if (act === 'delete') {
    if (entryCapabilities(entry).kind === 'pending') composer.deletePending(entry, rowEl);
    else composer.deleteEntry(entry, rowEl);
    return;
  }
  openComposerForHistoryRow(entry, act);
});

document.addEventListener('keydown', (e) => {
  const row = e.target.closest?.('.history-item[tabindex]');
  if (!row || composer.isOpen()) return;
  const entry = findHistoryRowEntry(row);
  if (!entry) return;
  const cap = entryCapabilities(entry);
  if (e.key === 'Enter' && cap.canEdit) {
    e.preventDefault();
    openComposerForHistoryRow(entry, 'edit');
  } else if ((e.key === 'Delete' || e.key === 'Backspace') && cap.canDelete) {
    e.preventDefault();
    if (cap.kind === 'pending') composer.deletePending(entry, row);
    else composer.deleteEntry(entry, row);
  }
});

// Empty-state add button (rebuilt with the list, so delegated too)
document.addEventListener('click', (e) => {
  if (e.target.closest('#history-empty-add')) composer.open({ date: getHistoryDate() });
});

// ── Sync indicator ───────────────────────────────────────────────────
function showSyncIndicator() {
  let el = $('#sync-indicator');
  if (!el) {
    el = document.createElement('div');
    el.id = 'sync-indicator';
    el.className = 'sync-indicator';
    el.innerHTML = '<span class="sync-spinner"></span> Syncing\u2026';
    document.querySelector('.history-header')?.appendChild(el);
  }
  el.style.display = '';
}
function hideSyncIndicator() {
  const el = $('#sync-indicator');
  if (el) el.style.display = 'none';
}

// Reusable render helpers for cache refresh events
function renderMonthSummary(summary) {
  const totalEl = $('#month-total');
  if (totalEl) totalEl.textContent = `Total: ${formatHours(summary.total_hours)}`;
  const maxHours = Math.max(8, ...summary.days.map(d => d.total_hours));
  const today = todayDate();
  const daysEl = $('#month-days');
  if (daysEl) {
    daysEl.innerHTML = summary.days.map(d => {
      const dt = new Date(d.date + 'T12:00:00');
      const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
      const label = `${days[dt.getDay()]} ${dt.getDate()}`;
      const pct = Math.min(100, (d.total_hours / maxHours) * 100);
      const isToday = d.date === today;
      return `<div class="month-day-row${isToday ? ' is-today' : ''}" data-date="${escAttr(d.date)}">
        <span class="month-day-label">${label}</span>
        <div class="month-day-bar-wrap"><div class="month-day-bar" style="width:${pct}%"></div></div>
        <span class="month-day-hours">${formatHours(d.total_hours)}</span>
      </div>`;
    }).join('');
  }
}

function renderAnalysis(analysis) {
  $('#analysis-presence').textContent = formatHours(analysis.total_attendance_hours);
  $('#analysis-logged').textContent = formatHours(analysis.total_timesheet_hours);
  const gapEl = $('#analysis-gap');
  if (gapEl) {
    gapEl.textContent = (analysis.gap_hours >= 0 ? '+' : '') + formatHours(Math.abs(analysis.gap_hours));
    gapEl.className = `analysis-stat-value ${analysis.gap_hours > 0.25 ? 'text-warning' : analysis.gap_hours < -0.25 ? 'text-danger' : 'text-success'}`;
  }
  const suggestionsEl = $('#analysis-suggestions');
  if (suggestionsEl) {
    suggestionsEl.innerHTML = analysis.suggestions.map(s => renderSuggestion(s)).join('');
  }
}

// Date navigation
async function goToHistoryDate(dateStr) {
  store.setState({ historyDate: dateStr, historyMode: 'day' });
  // Close analysis panel
  const panel = $('#analysis-panel');
  if (panel) panel.style.display = 'none';
  showSyncIndicator();
  try {
    const entries = await api.getEntriesForDate(dateStr);
    store.setState({ todayEntries: entries });
    store.setState({ todayPending: await refreshPendingFor(dateStr) });
  } catch (_) {}
  // If entries came from cache, the indicator stays until entries_refreshed event
  // If entries came from Odoo (no cache), hide immediately
  hideSyncIndicator();
}

$('#btn-history-prev')?.addEventListener('click', () => goToHistoryDate(addDays(getHistoryDate(), -1)));
$('#btn-history-next')?.addEventListener('click', () => {
  const next = addDays(getHistoryDate(), 1);
  if (next <= todayDate()) goToHistoryDate(next);
});

// Toggle to month view
$('#btn-history-toggle-month')?.addEventListener('click', async () => {
  const state = store.getState();
  if (state.historyMode === 'month') {
    store.setState({ historyMode: 'day' });
    return;
  }
  const date = getHistoryDate();
  const d = new Date(date + 'T12:00:00');
  const year = d.getFullYear();
  const month = d.getMonth() + 1;
  store.setState({ historyMode: 'month', monthYear: { year, month } });
  loadMonthView(year, month);
});

$('#btn-back-to-day')?.addEventListener('click', () => store.setState({ historyMode: 'day' }));

async function loadMonthView(year, month) {
  const titleEl = $('#month-title');
  if (titleEl) titleEl.textContent = `${getMonthName(month)} ${year}`;
  showSyncIndicator();
  try {
    const summary = await api.getMonthlySummary(year, month);
    renderMonthSummary(summary);
  } catch (err) { showToast(String(err)); }
  hideSyncIndicator();
}

// Click on month day -> navigate to that day
document.addEventListener('click', (e) => {
  const row = e.target.closest('.month-day-row[data-date]');
  if (!row) return;
  goToHistoryDate(row.dataset.date);
});

// Month navigation arrows
$('#btn-month-prev')?.addEventListener('click', () => {
  const my = store.getState().monthYear;
  if (!my) return;
  let { year, month } = my;
  month--;
  if (month < 1) { month = 12; year--; }
  store.setState({ monthYear: { year, month } });
  loadMonthView(year, month);
});

$('#btn-month-next')?.addEventListener('click', () => {
  const my = store.getState().monthYear;
  if (!my) return;
  let { year, month } = my;
  month++;
  if (month > 12) { month = 1; year++; }
  store.setState({ monthYear: { year, month } });
  loadMonthView(year, month);
});

// Manual entry — scoped to the date the history view is showing, so past
// days work with no extra step.
$('#btn-add-entry')?.addEventListener('click', () => composer.open({ date: getHistoryDate() }));

$('#btn-analyze-day')?.addEventListener('click', async () => {
  const panel = $('#analysis-panel');
  const btn = $('#btn-analyze-day');
  btn.disabled = true;
  btn.textContent = 'Analyzing...';
  try {
    const analysis = await api.getDayAnalysis(getHistoryDate());
    renderAnalysis(analysis);
    panel.style.display = '';
  } catch (err) { showToast(String(err)); }
  btn.disabled = false;
  btn.textContent = 'Analyze Day';
});

function renderSuggestion(s) {
  const iconMap = {
    add_time: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/></svg>',
    split_gap: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="2" x2="12" y2="22"/><polyline points="4 8 12 2 20 8"/><polyline points="4 16 12 22 20 16"/></svg>',
    missing_recurring: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--warning)" stroke-width="2.5" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>',
    all_good: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--success)" stroke-width="2.5" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><polyline points="8 12 11 15 16 9"/></svg>',
    info: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
  };
  const icon = iconMap[s.suggestion_type] || iconMap.info;
  const hasAction = (s.suggestion_type === 'add_time' || s.suggestion_type === 'missing_recurring') && s.task_id;

  let actionBtn = '';
  if (hasAction) {
    actionBtn = `<button class="analysis-action-btn" data-suggestion-id="${escAttr(s.id)}"
      data-task-id="${s.task_id}" data-project-id="${s.project_id || 0}"
      data-task-name="${escAttr(s.task_name)}" data-project-name="${escAttr(s.project_name)}"
      data-hours="${s.hours}" data-description="${escAttr(s.description || s.task_name)}">Apply</button>`;
  } else if (s.suggestion_type === 'split_gap' && s.hours) {
    actionBtn = `<button class="analysis-action-btn analysis-action-split" data-suggestion-id="${escAttr(s.id)}"
      data-split-hours="${s.hours}">Split</button>`;
  }

  return `<div class="analysis-suggestion" id="suggestion-${escAttr(s.id)}">
    <div class="suggestion-icon">${icon}</div>
    <div class="suggestion-text">
      <div class="suggestion-message">${esc(s.message)}</div>
      ${s.detail ? `<div class="suggestion-detail">${esc(s.detail)}</div>` : ''}
    </div>
    ${actionBtn}
  </div>`;
}

// Handle suggestion action button clicks
document.addEventListener('click', async (e) => {
  const btn = e.target.closest('.analysis-action-btn');
  if (!btn) return;

  // Confirmation pattern: first click = "Sure?", second click = execute
  if (!btn.dataset.confirmed) {
    btn.dataset.confirmed = 'true';
    btn.textContent = 'Sure?';
    btn.classList.add('confirming');
    setTimeout(() => {
      if (btn.dataset.confirmed) {
        delete btn.dataset.confirmed;
        btn.textContent = btn.classList.contains('analysis-action-split') ? 'Split' : 'Apply';
        btn.classList.remove('confirming');
      }
    }, 3000);
    return;
  }

  delete btn.dataset.confirmed;
  btn.classList.remove('confirming');

  if (btn.classList.contains('analysis-action-split')) {
    // Split gap: distribute proportionally across today's entries
    btn.disabled = true;
    btn.textContent = '...';
    try {
      const analysis = await api.getDayAnalysis(todayDate());
      const gap = analysis.gap_hours;
      const total = analysis.total_timesheet_hours;
      for (const entry of analysis.timesheet_blocks) {
        if (!entry.task_id) continue;
        const proportion = total > 0 ? entry.hours / total : 1 / analysis.timesheet_blocks.length;
        const share = Math.round(gap * proportion * 4) / 4;
        if (share > 0) {
          await api.logTime(entry.task_id, entry.project_id || 0, entry.task_name, entry.project_name, entry.task_name, share, todayDate());
        }
      }
      showToast('Gap distributed across tasks', 'success');
      markSuggestionDone(btn);
      refreshTodayEntries();
    } catch (err) {
      btn.textContent = 'Retry';
      btn.disabled = false;
      showToast(String(err));
    }
    return;
  }

  // Add time action
  const taskId = parseInt(btn.dataset.taskId);
  const projectId = parseInt(btn.dataset.projectId) || 0;
  const taskName = btn.dataset.taskName;
  const projectName = btn.dataset.projectName;
  const hours = parseFloat(btn.dataset.hours);
  const description = btn.dataset.description || taskName;

  btn.disabled = true;
  btn.textContent = '...';
  try {
    await api.logTime(taskId, projectId, taskName, projectName, description, hours, todayDate());
    showToast(`Added ${formatHours(hours)} to ${taskName}`, 'success');
    markSuggestionDone(btn);
    refreshTodayEntries();
  } catch (err) {
    btn.textContent = 'Retry';
    btn.disabled = false;
    showToast(String(err));
  }
});

function markSuggestionDone(btn) {
  const card = btn.closest('.analysis-suggestion');
  if (card) {
    card.classList.add('suggestion-done');
    btn.outerHTML = '<span class="suggestion-done-label">Done</span>';
  }
}

$('#btn-close-analysis')?.addEventListener('click', () => {
  $('#analysis-panel').style.display = 'none';
});

$('#btn-sync')?.addEventListener('click', async () => {
  try {
    const result = await api.syncPending();
    const parts = [];
    if (result.synced > 0) parts.push(`${result.synced} synced`);
    if (result.duplicates > 0) parts.push(`${result.duplicates} duplicates found`);
    if (result.rejected > 0) parts.push(`${result.rejected} rejected`);
    if (result.failed > 0) parts.push(`${result.failed} failed`);
    // sync_pending returns a ZERO-FILLED result (not an error) when another
    // sync already holds the lock, so key the message off remaining /
    // needs_review — `synced === 0` would report "Nothing to sync" while
    // entries are in fact still queued.
    const stillQueued = (result.remaining || 0) + (result.needs_review || 0);
    const msg = parts.length > 0
      ? parts.join(', ')
      : (stillQueued > 0 ? `${stillQueued} still queued — sync already running` : 'Nothing to sync');
    showToast(msg, result.synced > 0 ? 'success' : (result.needs_review > 0 ? 'warning' : 'info'));
    const syncStatus = await api.getSyncStatus();
    store.setState({ syncStatus });
    if (result.needs_review > 0) showSyncReviewPanel();
  } catch (err) { showToast(String(err)); }
});

// ── Sync Review Panel ────────────────────────────────────────────────

async function showSyncReviewPanel() {
  let panel = $('#sync-review-panel');
  if (!panel) {
    panel = document.createElement('div');
    panel.id = 'sync-review-panel';
    panel.className = 'analysis-panel';
    document.querySelector('.history-view')?.appendChild(panel);
  }
  panel.style.display = '';
  panel.innerHTML = '<div class="analysis-header"><span>Sync Review</span></div><div class="analysis-body"><span class="sync-spinner"></span> Loading...</div>';

  try {
    const entries = await api.getReviewEntries();
    if (entries.length === 0) {
      panel.innerHTML = '<div class="analysis-header"><span>Sync Review</span><button class="btn btn-sm btn-secondary" id="btn-close-sync-review">Close</button></div><div class="analysis-body"><p>All clear — no entries need review.</p></div>';
      $('#btn-close-sync-review')?.addEventListener('click', () => { panel.style.display = 'none'; });
      return;
    }

    const rows = entries.map(e => {
      const statusLabel = e.status === 'duplicate' ? 'Duplicate found'
        : e.status === 'rejected' ? 'Rejected by Odoo'
        : `Failed (${e.retry_count} retries)`;
      const statusClass = e.status === 'duplicate' ? 'warning' : 'error';
      return `<div class="sync-review-entry" data-entry-id="${e.id}">
        <div class="sync-review-info">
          <span class="sync-review-status ${statusClass}">${statusLabel}</span>
          <span class="sync-review-task">${escapeHtml(e.task_name || ('Task #' + e.task_id))}</span>
          ${e.description ? `<span class="sync-review-meta">${escapeHtml(e.description)}</span>` : ''}
          <span class="sync-review-meta">${e.date} &middot; ${e.duration_hours.toFixed(2)}h</span>
          ${e.last_error ? `<span class="sync-review-error">${escapeHtml(e.last_error)}</span>` : ''}
        </div>
        <div class="sync-review-actions">
          <button class="btn btn-sm btn-secondary" data-action="edit">Edit &amp; retry</button>
          ${e.status === 'duplicate'
            ? `<button class="btn btn-sm btn-secondary" data-action="skip">Skip (already in Odoo)</button>
               <button class="btn btn-sm btn-primary" data-action="force">Send anyway (creates a duplicate)</button>`
            : `<button class="btn btn-sm btn-primary" data-action="force">Retry</button>`}
          <button class="btn btn-sm btn-danger" data-action="discard">Discard</button>
        </div>
      </div>`;
    }).join('');

    panel.innerHTML = `<div class="analysis-header"><span>Sync Review (${entries.length})</span><button class="btn btn-sm btn-secondary" id="btn-close-sync-review">Close</button></div><div class="analysis-body sync-review-list">${rows}</div>`;

    $('#btn-close-sync-review')?.addEventListener('click', () => { panel.style.display = 'none'; });

    panel.querySelectorAll('.sync-review-actions button').forEach(btn => {
      btn.addEventListener('click', async () => {
        const entryEl = btn.closest('.sync-review-entry');
        const entryId = parseInt(entryEl.dataset.entryId, 10);
        const action = btn.dataset.action;

        // REPAIR mode. resolve_sync_entry can only skip/force/discard — it
        // cannot change a queued row's payload, so re-pointing a rejected
        // "private task" entry at a different task is otherwise impossible
        // and the row sits in the queue forever.
        if (action === 'edit') {
          const src = entries.find(x => x.id === entryId);
          if (!src) return;
          composer.open({
            mode: 'repair',
            pendingId: entryId,
            date: src.date,
            taskId: src.task_id,
            taskName: src.task_name || `Task #${src.task_id}`,
            projectId: src.project_id || 0,
            projectName: src.project_name || '',
            description: src.description || '',
            durationHours: src.duration_hours,
          });
          return;
        }

        try {
          await api.resolveSyncEntry(entryId, action);
          entryEl.remove();
          const ss = await api.getSyncStatus();
          store.setState({ syncStatus: ss });
          showToast(`Entry ${action === 'discard' ? 'discarded' : action === 'skip' ? 'skipped' : 'queued for retry'}`, 'success');
          if (panel.querySelectorAll('.sync-review-entry').length === 0) {
            panel.innerHTML = '<div class="analysis-header"><span>Sync Review</span><button class="btn btn-sm btn-secondary" id="btn-close-sync-review">Close</button></div><div class="analysis-body"><p>All clear!</p></div>';
            $('#btn-close-sync-review')?.addEventListener('click', () => { panel.style.display = 'none'; });
          }
        } catch (err) { showToast(String(err)); }
      });
    });
  } catch (err) {
    panel.innerHTML = `<div class="analysis-header"><span>Sync Review</span></div><div class="analysis-body"><p>Error loading entries: ${escapeHtml(String(err))}</p></div>`;
  }
}

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

// ── Settings View ─────────────────────────────────────────────────────

store.subscribe((state) => {
  const urlEl = $('#settings-url');
  const userEl = $('#settings-username');
  if (urlEl) urlEl.textContent = state.auth.url || 'Not connected';
  if (userEl) userEl.textContent = state.auth.username || '';
});

$('#btn-disconnect')?.addEventListener('click', async () => {
  try {
    await api.logout();
    cachedOdooUrl = '';
    store.setState({
      auth: { authenticated: false, username: null, url: null },
      timer: { is_running: false, elapsed_secs: 0 },
      attendance: { is_checked_in: false, attendance_id: null, check_in_time: null },
      stoppedTimer: null,
      tasks: [], recentTasks: [], myTasks: [], todayEntries: [],
    });
    navigateTo('login');
  } catch (err) { showToast(String(err)); }
});

// Check for update button
$('#btn-check-update')?.addEventListener('click', async () => {
  const btn = $('#btn-check-update');
  const status = $('#update-status');
  btn.disabled = true;
  btn.textContent = 'Checking\u2026';
  status.textContent = 'Checking for updates\u2026';
  try {
    const info = await api.checkForUpdate();
    if (info.available) {
      status.textContent = `v${info.version} available!`;
      status.style.color = 'var(--primary)';
      showUpdateModal(info);
    } else {
      status.textContent = 'You\u2019re up to date';
      status.style.color = 'var(--success, #22c55e)';
    }
  } catch (e) {
    status.textContent = 'Failed to check: ' + String(e).slice(0, 60);
    status.style.color = 'var(--danger, #ef4444)';
  }
  btn.disabled = false;
  btn.textContent = 'Check for update';
});

// Theme selector
$('#setting-theme')?.addEventListener('change', (e) => {
  const theme = e.target.value;
  if (theme === 'dark') {
    document.documentElement.removeAttribute('data-theme');
  } else {
    document.documentElement.setAttribute('data-theme', theme);
  }
  localStorage.setItem('pointeuse-theme', theme);
  showToast(`Theme: ${theme}`, 'success');
});

// Autostart toggle
$('#setting-autostart')?.addEventListener('change', async (e) => {
  try {
    await api.setAutostartEnabled(e.target.checked);
    showToast(e.target.checked ? 'Autostart enabled' : 'Autostart disabled', 'success');
  } catch (err) {
    e.target.checked = !e.target.checked;
    showToast(String(err));
  }
});

// Test popup button
$('#btn-test-popup')?.addEventListener('click', async () => {
  try { await api.testReminderPopup(); } catch (err) { showToast(String(err)); }
});

// Notification channel selector (Android only)
if (/android/i.test(navigator.userAgent)) {
  const channelRow = $('#setting-notif-channel-row');
  if (channelRow) channelRow.style.display = '';

  // Action types are registered natively in MainActivity.onCreate() via NotificationActionSetup
}
$('#setting-reminder-channel')?.addEventListener('change', async (e) => {
  try { await api.setReminderChannel(e.target.value); } catch (err) { showToast(String(err)); }
});

// Reminder interval setting
$('#setting-reminder-interval')?.addEventListener('change', async (e) => {
  const minutes = parseInt(e.target.value);
  try {
    await api.setReminderInterval(minutes);
    store.setState({ reminderInterval: minutes });
  } catch (err) { showToast(String(err)); }
});

// ── Quick-Switch Settings ─────────────────────────────────────────────

$('#setting-quickswitch-mode')?.addEventListener('change', async (e) => {
  const mode = e.target.value;
  try {
    await api.setQuickswitchMode(mode);
    store.setState({ quickswitchMode: mode });
    const manualCfg = document.getElementById('quickswitch-manual-config');
    if (manualCfg) manualCfg.style.display = mode === 'manual' ? '' : 'none';
  } catch (err) { showToast(String(err)); }
});

function renderPinnedQuickswitchItems() {
  const list = document.getElementById('quickswitch-pinned-list');
  if (!list) return;
  const items = store.getState().quickswitchItems || [];
  if (items.length === 0) {
    list.innerHTML = '<div class="empty-state" style="padding:8px"><p style="font-size:11px">No pinned tasks yet</p></div>';
    return;
  }
  list.innerHTML = items.map((item, i) => `
    <div class="quickswitch-pin-item" data-index="${i}">
      <span class="quickswitch-pin-slot ${item.slot}">${item.slot === 'main' ? 'L' : 'S'}</span>
      <span class="quickswitch-pin-name">${esc(item.task_name)}</span>
      <span class="quickswitch-pin-project">${esc(item.project_name)}</span>
      <button class="quickswitch-pin-toggle" data-index="${i}" title="Toggle size">${item.slot === 'main' ? '&#9660;' : '&#9650;'}</button>
      <button class="quickswitch-pin-remove" data-index="${i}" title="Remove">&times;</button>
    </div>
  `).join('');
}

document.addEventListener('click', async (e) => {
  const removeBtn = e.target.closest('.quickswitch-pin-remove');
  if (removeBtn) {
    const idx = parseInt(removeBtn.dataset.index);
    const items = [...(store.getState().quickswitchItems || [])];
    items.splice(idx, 1);
    store.setState({ quickswitchItems: items });
    await api.setQuickswitchItems(items);
    renderPinnedQuickswitchItems();
    return;
  }
  const toggleBtn = e.target.closest('.quickswitch-pin-toggle');
  if (toggleBtn) {
    const idx = parseInt(toggleBtn.dataset.index);
    const items = [...(store.getState().quickswitchItems || [])];
    // Count current mains and smalls
    const mainCount = items.filter((it, ii) => it.slot === 'main' && ii !== idx).length;
    const smallCount = items.filter((it, ii) => it.slot === 'small' && ii !== idx).length;
    if (items[idx].slot === 'main') {
      if (smallCount < 3) { items[idx] = { ...items[idx], slot: 'small' }; }
      else { showToast('Max 3 small slots'); return; }
    } else {
      if (mainCount < 4) { items[idx] = { ...items[idx], slot: 'main' }; }
      else { showToast('Max 4 main slots'); return; }
    }
    store.setState({ quickswitchItems: items });
    await api.setQuickswitchItems(items);
    renderPinnedQuickswitchItems();
  }
});

$('#btn-add-quickswitch')?.addEventListener('click', () => {
  // Open a mini task picker for adding a quickswitch item
  showQuickswitchPicker();
});

function showQuickswitchPicker() {
  // Reuse the task picker popup in a special mode
  showTaskPrompt('quickswitch');
}

// ── Default Task Setting ─────────────────────────────────────────────

async function renderDefaultTask() {
  const dt = await api.getDefaultTask().catch(() => null);
  const currentEl = $('#default-task-current');
  const btnSet = $('#btn-set-default-task');
  if (dt) {
    $('#default-task-name').textContent = dt.task_name;
    $('#default-task-project').textContent = dt.project_name || '';
    if (currentEl) currentEl.style.display = '';
    if (btnSet) btnSet.textContent = 'Change task...';
  } else {
    if (currentEl) currentEl.style.display = 'none';
    if (btnSet) btnSet.textContent = 'Choose a task...';
  }
}

$('#btn-set-default-task')?.addEventListener('click', () => {
  showTaskPrompt('default_task');
});

$('#btn-clear-default-task')?.addEventListener('click', async () => {
  try {
    await api.clearDefaultTask();
    renderDefaultTask();
    showToast('Default task cleared', 'success');
  } catch (err) { showToast(String(err)); }
});

// ── Hide Done Tasks Setting ───────────────────────────────────────────

$('#setting-hide-done')?.addEventListener('change', async (e) => {
  const hide = e.target.checked;
  try {
    await api.setHideDoneTasks(hide);
    store.setState({ hideDoneTasks: hide });
  } catch (err) { showToast(String(err)); }
});

// ── Cross-device Sync Settings ────────────────────────────────────────

$('#setting-device-sync')?.addEventListener('change', async (e) => {
  const enabled = e.target.checked;
  try {
    await api.setDeviceSyncEnabled(enabled);
    store.setState({ deviceSyncEnabled: enabled });
    showToast(enabled ? 'Timer sync enabled' : 'Timer sync disabled', 'success');
  } catch (err) {
    e.target.checked = !enabled;
    showToast(String(err));
  }
});

$('#setting-device-label')?.addEventListener('change', async (e) => {
  const label = e.target.value.trim();
  if (!label) {
    e.target.value = store.getState().device.label;
    return;
  }
  try {
    await api.setDeviceLabel(label);
    const device = await api.getDeviceIdentity();
    store.setState({ device });
  } catch (err) { showToast(String(err)); }
});

// ── Task Picker Popup ─────────────────────────────────────────────────

const taskPickerPopup = $('#task-picker-popup');
let taskPickerSearchTimeout;
let taskPickerMode = 'checkin'; // 'checkin', 'switch', or 'quickswitch'

async function showTaskPrompt(mode = 'checkin') {
  taskPickerMode = mode;
  const titleEl = $('#task-picker-title');
  if (titleEl) {
    titleEl.textContent = mode === 'switch' ? 'Switch to which task?'
      : mode === 'quickswitch' ? 'Pin a task to Quick Switch'
      : mode === 'default_task' ? 'Choose default task'
      : 'What are you working on?';
  }
  const dismissBtn = $('#task-picker-dismiss');
  if (dismissBtn) {
    dismissBtn.textContent = mode === 'switch' || mode === 'quickswitch' || mode === 'default_task' ? 'Cancel' : 'Skip for now';
  }
  taskPickerPopup?.classList.add('visible');
  const searchEl = $('#task-picker-search');
  if (searchEl) { searchEl.value = ''; searchEl.focus(); }
  // Load suggested tasks + all my tasks, split into sections
  try {
    const [suggested, myTasks] = await Promise.all([
      api.getSuggestedTasks().catch(() => []),
      api.getMyTasks().catch(() => []),
    ]);
    // Suggested = tasks with a score reason
    const suggestedIds = new Set(suggested.filter(t => t.reason).map(t => t.id));
    const suggestedList = suggested.filter(t => suggestedIds.has(t.id));
    // Other = my tasks not in the suggested set
    const otherList = myTasks.filter(t => !suggestedIds.has(t.id));
    renderTaskList($('#task-picker-list'), suggestedList, { suggested: true, otherTasks: otherList });
  } catch (_) {
    try {
      const tasks = await api.getMyTasks();
      renderTaskList($('#task-picker-list'), tasks);
    } catch (__) {}
  }

  // Populate recent chips for fast one-tap access
  const recentsContainer = $('#task-picker-recents');
  if (recentsContainer) {
    try {
      const recent = await api.getRecentTasks();
      const top = recent.slice(0, 6);
      recentsContainer.innerHTML = top.map(t =>
        `<div class="recent-chip" data-task-id="${t.id}" data-task-name="${escAttr(t.name)}"
          data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
          <svg class="recent-chip-icon" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 8v4l3 3"/><circle cx="12" cy="12" r="10"/></svg>
          ${esc(t.name)}
        </div>`
      ).join('');
    } catch { recentsContainer.innerHTML = ''; }
    recentsContainer.style.display = '';
  }
}

function hideTaskPrompt() {
  taskPickerPopup?.classList.remove('visible');
}

$('#task-picker-dismiss')?.addEventListener('click', hideTaskPrompt);
$('#task-picker-backdrop')?.addEventListener('click', hideTaskPrompt);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && taskPickerPopup?.classList.contains('visible')) hideTaskPrompt();
});

$('#task-picker-search')?.addEventListener('input', (e) => {
  clearTimeout(taskPickerSearchTimeout);
  const q = e.target.value.trim();
  const recentsContainer = $('#task-picker-recents');
  if (recentsContainer) recentsContainer.style.display = q ? 'none' : '';
  if (q.length > 0) {
    taskPickerSearchTimeout = setTimeout(async () => {
      try {
        const tasks = await api.searchTasks(q, null);
        renderTaskList($('#task-picker-list'), tasks);
      } catch (_) {}
    }, 300);
  } else {
    // Reload suggested + all tasks
    (async () => {
      try {
        const [suggested, myTasks] = await Promise.all([
          api.getSuggestedTasks().catch(() => []),
          api.getMyTasks().catch(() => []),
        ]);
        const suggestedIds = new Set(suggested.filter(t => t.reason).map(t => t.id));
        const suggestedList = suggested.filter(t => suggestedIds.has(t.id));
        const otherList = myTasks.filter(t => !suggestedIds.has(t.id));
        renderTaskList($('#task-picker-list'), suggestedList, { suggested: true, otherTasks: otherList });
      } catch (_) {
        try {
          const tasks = await api.getMyTasks();
          renderTaskList($('#task-picker-list'), tasks);
        } catch (__) {}
      }
    })();
  }
});

// Handle task selection from picker popup
document.addEventListener('click', async (e) => {
  if (!taskPickerPopup?.classList.contains('visible')) return;
  const item = e.target.closest('#task-picker-list .task-item[data-task-id]') || e.target.closest('.recent-chip[data-task-id]');
  if (!item) return;

  const taskId = parseInt(item.dataset.taskId);
  const taskName = item.dataset.taskName;
  const projectId = parseInt(item.dataset.projectId) || 0;
  const projectName = item.dataset.projectName;

  try {
    // Quick-switch pin mode — add to pinned items, don't start timer
    if (taskPickerMode === 'quickswitch') {
      const items = [...(store.getState().quickswitchItems || [])];
      // Prevent duplicates
      if (items.some(it => it.task_id === taskId)) {
        showToast('Task already pinned');
        hideTaskPrompt();
        return;
      }
      // Determine slot: first 4 go to main, next 3 to small
      const mainCount = items.filter(it => it.slot === 'main').length;
      const smallCount = items.filter(it => it.slot === 'small').length;
      let slot = 'main';
      if (mainCount >= 4) slot = 'small';
      if (mainCount >= 4 && smallCount >= 3) {
        showToast('Max 7 quick-switch items');
        hideTaskPrompt();
        return;
      }
      items.push({ task_id: taskId, task_name: taskName, project_id: projectId, project_name: projectName, slot });
      store.setState({ quickswitchItems: items });
      await api.setQuickswitchItems(items);
      renderPinnedQuickswitchItems();
      hideTaskPrompt();
      showToast('Task pinned to Quick Switch', 'success');
      return;
    }

    // If setting default task, save and return
    if (taskPickerMode === 'default_task') {
      await api.setDefaultTask(taskId, taskName, projectId, projectName);
      hideTaskPrompt();
      renderDefaultTask();
      showToast('Default task set', 'success');
      return;
    }

    // If switching, stop current timer first (auto-log it)
    if (taskPickerMode === 'switch') {
      const timer = store.getState().timer;
      if (timer.is_running) {
        const stopped = await api.stopTimer();
        const hours = stopped.elapsed_secs / 3600;
        await api.logTime(stopped.task_id, stopped.project_id || 0, stopped.task_name, stopped.project_name, stopped.task_name, hours, todayDate());
        showToast(`Logged ${formatTime(stopped.elapsed_secs)} for ${stopped.task_name}`, 'success');
      }
    }

    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    hideTaskPrompt();
    navigateTo('timer');
  } catch (err) { showToast(String(err)); }
});

// ── Timer Polling ─────────────────────────────────────────────────────

let timerInterval = null;

function startPolling() {
  if (timerInterval) return;
  timerInterval = setInterval(async () => {
    try {
      const t = await api.getTimerState();
      store.setState({ timer: t });
    } catch (_) {}
  }, 1000);
}

function stopPolling() {
  clearInterval(timerInterval);
  timerInterval = null;
}

store.subscribe((state, prev) => {
  if (state.timer.is_running && !prev.timer?.is_running) startPolling();
  if (!state.timer.is_running && prev.timer?.is_running) stopPolling();
});

// ── Periodic sync ─────────────────────────────────────────────────────

setInterval(async () => {
  if (store.getState().auth.authenticated) {
    try {
      const result = await api.syncPending();
      const ss = await api.getSyncStatus();
      store.setState({ syncStatus: ss });
      if (result.needs_review > 0) {
        showToast(`${result.needs_review} sync entries need review`, 'warning');
      }
    } catch (_) {}
  }
}, 5 * 60 * 1000);

// ── Data refresh helpers ──────────────────────────────────────────────

async function refreshTimerState() {
  try {
    const t = await api.getTimerState();
    store.setState({ timer: t });
  } catch (_) {}
}

async function refreshTodayEntries() {
  try {
    const entries = await api.getTodayEntries();
    store.setState({ todayEntries: entries });
  } catch (_) {}
}

async function refreshAttendance() {
  try {
    const att = await api.getAttendanceStatus();
    store.setState({ attendance: att });
  } catch (_) {}
}

function refreshTheme() {
  const theme = localStorage.getItem('pointeuse-theme') || 'dark';
  const sel = $('#setting-theme');
  if (sel) sel.value = theme;
}

async function refreshAutostart() {
  try {
    const enabled = await api.getAutostartEnabled();
    const el = $('#setting-autostart');
    if (el) el.checked = enabled;
  } catch (_) {}
}

async function refreshReminderInterval() {
  try {
    const minutes = await api.getReminderInterval();
    store.setState({ reminderInterval: minutes });
    const sel = $('#setting-reminder-interval');
    if (sel) sel.value = String(minutes);
  } catch (_) {}
  // Also load channel preference (Android only)
  if (/android/i.test(navigator.userAgent)) {
    try {
      const channel = await api.getReminderChannel();
      const chSel = $('#setting-reminder-channel');
      if (chSel) chSel.value = channel;
    } catch (_) {}
  }
}

async function refreshQuickswitchSettings() {
  try {
    const mode = await api.getQuickswitchMode();
    const items = await api.getQuickswitchItems();
    store.setState({ quickswitchMode: mode, quickswitchItems: items });
    const sel = $('#setting-quickswitch-mode');
    if (sel) sel.value = mode;
    const manualCfg = document.getElementById('quickswitch-manual-config');
    if (manualCfg) manualCfg.style.display = mode === 'manual' ? '' : 'none';
    renderPinnedQuickswitchItems();
  } catch (_) {}
}

async function refreshHideDoneTasks() {
  try {
    const hide = await api.getHideDoneTasks();
    store.setState({ hideDoneTasks: hide });
    const el = $('#setting-hide-done');
    if (el) el.checked = hide;
  } catch (_) {}
}

async function refreshDeviceSync() {
  try {
    const [device, enabled] = await Promise.all([
      api.getDeviceIdentity(),
      api.getDeviceSyncEnabled(),
    ]);
    store.setState({ device, deviceSyncEnabled: enabled });
    const toggle = $('#setting-device-sync');
    if (toggle) toggle.checked = enabled;
    const label = $('#setting-device-label');
    if (label) label.value = device.label || '';
  } catch (_) {}
}

async function refreshAll() {
  await Promise.all([
    refreshTimerState(),
    refreshTodayEntries(),
    refreshAttendance(),
    refreshReminderInterval(),
    refreshQuickswitchSettings(),
    refreshHideDoneTasks(),
    refreshDeviceSync(),
  ]);
}

store.subscribe((state, prev) => {
  if (state.view !== prev.view) {
    if (state.view === 'timer') { refreshTodayEntries(); renderTimerBreakdown(); }
    if (state.view === 'history') {
      // Keep current date if already set, otherwise go to today
      if (!store.getState().historyDate) {
        goToHistoryDate(todayDate());
      } else {
        goToHistoryDate(store.getState().historyDate);
      }
    }
    if (state.view === 'tasks') loadTasksForTab();
    if (state.view === 'timer') refreshAttendance();
    if (state.view === 'settings') { refreshReminderInterval(); refreshAutostart(); refreshTheme(); refreshQuickswitchSettings(); refreshHideDoneTasks(); renderDefaultTask(); }
  }
});

// ── Bootstrap ─────────────────────────────────────────────────────────

async function init() {
  navigateTo('login');

  try {
    const auth = await api.checkAuth();
    store.setState({ auth });
    if (auth.authenticated) {
      navigateTo('timer');
      await refreshAll();
      renderTimerBreakdown();
      return;
    }
  } catch (e) {
    console.error('Init error:', e);
  }

  // Prepopulate login form
  try {
    const saved = await api.getSavedConnection();
    if (saved.url) $('#login-url').value = saved.url;
    if (saved.username) $('#login-username').value = saved.username;
    if (saved.username) $('#login-password').focus();
  } catch (_) {}
}

// Only bootstrap in the main window — the reminder popup loads index.html
// temporarily before replacing its content, so we must skip init there.
// On mobile (Android/iOS) there's only one webview, so always init.
const _isMainWindow = (() => {
  try {
    // On mobile, getCurrentWindow may not have a label or may not exist
    const win = window.__TAURI__?.window?.getCurrentWindow?.();
    if (!win || !win.label) return true; // mobile or unknown → init
    return win.label === 'main';
  } catch (_) { return true; }
})();

if (_isMainWindow) {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
}

// ── Update checker ───────────────────────────────────────────────────

async function checkForAppUpdate() {
  try {
    const info = await api.checkForUpdate();
    if (info.available) showUpdateModal(info);
  } catch (_) {}
}

function showUpdateModal(info) {
  // Don't show again if dismissed this session for this version
  if (window._dismissedUpdateVersion === info.version) return;

  let overlay = document.getElementById('update-modal-overlay');
  if (overlay) overlay.remove();

  overlay = document.createElement('div');
  overlay.id = 'update-modal-overlay';
  overlay.className = 'update-modal-overlay';

  const body = (info.body || '').replace(/\n/g, '<br>');
  overlay.innerHTML = `
    <div class="update-modal">
      <div class="update-modal-header">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--primary)" stroke-width="2" stroke-linecap="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
          <polyline points="7 10 12 15 17 10"/>
          <line x1="12" y1="15" x2="12" y2="3"/>
        </svg>
        <span>Update Available</span>
      </div>
      <div class="update-modal-body">
        <p class="update-version">Version ${esc(info.version || '?')}</p>
        ${body ? `<div class="update-notes">${body}</div>` : ''}
      </div>
      <div class="update-modal-actions">
        <button class="btn btn-secondary btn-sm" id="update-dismiss">Later</button>
        <button class="btn btn-primary btn-sm" id="update-install">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
            <polyline points="7 10 12 15 17 10"/>
            <line x1="12" y1="15" x2="12" y2="3"/>
          </svg>
          Update & Restart
        </button>
      </div>
    </div>`;
  document.body.appendChild(overlay);

  document.getElementById('update-dismiss').addEventListener('click', () => {
    window._dismissedUpdateVersion = info.version;
    overlay.remove();
  });

  document.getElementById('update-install').addEventListener('click', async () => {
    const btn = document.getElementById('update-install');
    btn.disabled = true;
    btn.textContent = 'Downloading\u2026';
    try {
      await api.installUpdate();
    } catch (e) {
      btn.disabled = false;
      btn.textContent = 'Update & Restart';
      showToast('Update failed: ' + String(e));
    }
  });
}

// Check for updates 5 seconds after startup, then every 4 hours (main window only)
if (_isMainWindow) {
  setTimeout(checkForAppUpdate, 5000);
  setInterval(checkForAppUpdate, 4 * 60 * 60 * 1000);
}

// ==========================================================================
// NEW FEATURES — Kanban Board, Command Palette, Filter Chips, etc.
// ==========================================================================

// ── State Config Map ─────────────────────────────────────────────────

const STATE_CONFIG = {
  '01_in_progress':        { label: 'In Progress',        color: '#3b82f6' },
  '02_changes_requested':  { label: 'Changes Requested',  color: '#ef4444' },
  '03_approved':           { label: 'Approved',           color: '#a855f7' },
  '04_waiting_normal':     { label: 'Waiting',            color: '#f59e0b' },
  '1_done':                { label: 'Done',               color: '#22c55e' },
  '1_canceled':            { label: 'Canceled',           color: '#6b7280' },
};

const STATE_ORDER = [
  '01_in_progress',
  '02_changes_requested',
  '03_approved',
  '04_waiting_normal',
  '1_done',
  '1_canceled',
];

// ── Enhanced Task Card Rendering ─────────────────────────────────────

function getDeadlinePill(task) {
  if (!task.date_deadline) return '';
  const now = new Date();
  const deadline = new Date(task.date_deadline + 'T23:59:59');
  const diffMs = deadline - now;
  const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24));

  let text, cls;
  if (diffDays < 0) {
    text = `${Math.abs(diffDays)}d overdue`;
    cls = 'overdue';
  } else if (diffDays === 0) {
    text = 'Due today';
    cls = 'soon';
  } else if (diffDays <= 3) {
    text = `${diffDays}d left`;
    cls = 'soon';
  } else {
    text = `${diffDays}d left`;
    cls = '';
  }
  return `<span class="deadline-pill ${cls}">${text}</span>`;
}

function renderTaskCard(task) {
  const stateKey = task.state || task.kanban_state_label || '';
  const cfg = STATE_CONFIG[stateKey] || { label: '', color: 'var(--brand)' };
  const deadlinePill = getDeadlinePill(task);
  const projectName = task.project_name || 'No project';
  const isPriority = task.priority === '1' || task.priority === 1 || task.is_priority;
  const priorityStar = isPriority ? '<span class="priority-star">&#9733;</span>' : '';

  return `<div class="task-card" data-state="${escAttr(stateKey)}"
    data-task-id="${task.id}" data-task-name="${escAttr(task.name)}"
    data-project-id="${task.project_id || 0}" data-project-name="${escAttr(task.project_name || '')}">
    <div class="task-card-left">
      <span class="state-dot" style="background: ${cfg.color}"></span>
    </div>
    <div class="task-card-body">
      <div class="task-card-title">${esc(task.name)}</div>
      <div class="task-card-meta">
        <span class="project-badge">${esc(projectName)}</span>
        ${deadlinePill}
      </div>
    </div>
    <div class="task-card-right">
      ${priorityStar}
    </div>
  </div>`;
}

// ── Kanban Board View ────────────────────────────────────────────────

let kanbanMode = false;

function toggleKanbanMode() {
  kanbanMode = !kanbanMode;
  const taskList = $('#task-list');
  const kanbanEl = $('#task-kanban');
  const toggleBtn = $('#btn-toggle-kanban');

  if (kanbanMode) {
    if (taskList) taskList.style.display = 'none';
    if (kanbanEl) kanbanEl.style.display = '';
    if (toggleBtn) toggleBtn.classList.add('active');
    renderKanbanBoard();
  } else {
    if (taskList) taskList.style.display = '';
    if (kanbanEl) kanbanEl.style.display = 'none';
    if (toggleBtn) toggleBtn.classList.remove('active');
  }
}

$('#btn-toggle-kanban')?.addEventListener('click', toggleKanbanMode);

async function renderKanbanBoard() {
  const kanbanEl = $('#task-kanban');
  if (!kanbanEl) return;

  kanbanEl.innerHTML = '<div class="empty-state"><p>Loading tasks...</p></div>';

  let tasks = [];
  try {
    tasks = await api.getMyTasks();
  } catch (_) {
    try { tasks = await api.getRecentTasks(); } catch (__) {}
  }

  if (!tasks || tasks.length === 0) {
    kanbanEl.innerHTML = '<div class="empty-state"><p>No tasks to display</p></div>';
    return;
  }

  // Group by state
  const groups = {};
  for (const key of STATE_ORDER) {
    groups[key] = [];
  }
  groups['_other'] = [];

  for (const t of tasks) {
    const stateKey = t.state || '';
    if (groups[stateKey]) {
      groups[stateKey].push(t);
    } else {
      groups['_other'].push(t);
    }
  }

  let html = '';
  for (const key of STATE_ORDER) {
    const cfg = STATE_CONFIG[key];
    const columnTasks = groups[key];
    html += `<div class="kanban-column" data-state="${escAttr(key)}">
      <div class="kanban-column-header">
        <span class="kanban-column-dot" style="background: ${cfg.color}"></span>
        <span class="kanban-column-title">${cfg.label}</span>
        <span class="kanban-column-count">${columnTasks.length}</span>
      </div>
      <div class="kanban-column-body">`;

    if (columnTasks.length === 0) {
      html += '<div class="kanban-empty">No tasks</div>';
    } else {
      for (const t of columnTasks) {
        html += renderTaskCard(t);
      }
    }
    html += '</div></div>';
  }

  // If there are tasks with unknown states, add an "Other" column
  if (groups['_other'].length > 0) {
    html += `<div class="kanban-column">
      <div class="kanban-column-header">
        <span class="kanban-column-dot" style="background: var(--text-muted)"></span>
        <span class="kanban-column-title">Other</span>
        <span class="kanban-column-count">${groups['_other'].length}</span>
      </div>
      <div class="kanban-column-body">`;
    for (const t of groups['_other']) {
      html += renderTaskCard(t);
    }
    html += '</div></div>';
  }

  kanbanEl.innerHTML = html;
}

// Task card click handler (for kanban board cards)
document.addEventListener('click', async (e) => {
  const card = e.target.closest('.task-card[data-task-id]');
  if (!card) return;
  // Skip if inside the task picker popup
  if (card.closest('#task-picker-list')) return;

  const taskId = parseInt(card.dataset.taskId);
  const taskName = card.dataset.taskName;
  const projectId = parseInt(card.dataset.projectId) || 0;
  const projectName = card.dataset.projectName;

  try {
    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    navigateTo('timer');
  } catch (err) { showToast(String(err)); }
});

// Refresh kanban when entering tasks view in kanban mode
store.subscribe((state, prev) => {
  if (state.view === 'tasks' && state.view !== prev.view && kanbanMode) {
    renderKanbanBoard();
  }
});

// ── Command Palette (Ctrl+K / Cmd+K) ────────────────────────────────

let commandPaletteOpen = false;
let commandActiveIndex = -1;
let commandSearchTimeout;
let commandTasks = [];

function openCommandPalette() {
  const palette = $('#command-palette');
  if (!palette) return;
  palette.style.display = '';
  commandPaletteOpen = true;
  commandActiveIndex = -1;
  const searchInput = $('#command-search');
  if (searchInput) {
    searchInput.value = '';
    searchInput.focus();
  }
  // Show recent tasks initially
  loadCommandRecentTasks();
}

function closeCommandPalette() {
  const palette = $('#command-palette');
  if (!palette) return;
  palette.style.display = 'none';
  commandPaletteOpen = false;
  commandTasks = [];
  commandActiveIndex = -1;
}

async function loadCommandRecentTasks() {
  try {
    const tasks = await api.getRecentTasks();
    commandTasks = tasks || [];
    renderCommandResults(commandTasks, 'Recent Tasks');
  } catch (_) {
    renderCommandResults([], '');
  }
}

function renderCommandResults(tasks, label) {
  const resultsEl = $('#command-results');
  if (!resultsEl) return;

  if (!tasks || tasks.length === 0) {
    resultsEl.innerHTML = '<div class="empty-state"><p>No tasks found</p></div>';
    return;
  }

  // Group by project
  const groups = {};
  for (const t of tasks) {
    const proj = t.project_name || 'No project';
    if (!groups[proj]) groups[proj] = [];
    groups[proj].push(t);
  }

  let html = '';
  let idx = 0;
  const sortedKeys = Object.keys(groups).sort((a, b) => {
    if (a === 'No project') return 1;
    if (b === 'No project') return -1;
    return a.localeCompare(b);
  });

  for (const proj of sortedKeys) {
    html += `<div class="command-group-label">${esc(proj)}</div>`;
    for (const t of groups[proj]) {
      const stateKey = t.state || '';
      const cfg = STATE_CONFIG[stateKey] || { color: 'var(--brand)' };
      html += `<div class="command-item" data-idx="${idx}"
        data-task-id="${t.id}" data-task-name="${escAttr(t.name)}"
        data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
        <span class="command-item-dot" style="background: ${cfg.color}"></span>
        <span class="command-item-name">${esc(t.name)}</span>
        <span class="command-item-project">${esc(t.project_name || '')}</span>
      </div>`;
      idx++;
    }
  }

  resultsEl.innerHTML = html;
  commandActiveIndex = -1;
}

function updateCommandActive() {
  const items = $$('#command-results .command-item');
  items.forEach((el, i) => {
    el.classList.toggle('active', i === commandActiveIndex);
  });
  // Scroll active item into view
  if (commandActiveIndex >= 0 && items[commandActiveIndex]) {
    items[commandActiveIndex].scrollIntoView({ block: 'nearest' });
  }
}

async function selectCommandItem(el) {
  if (!el) return;
  const taskId = parseInt(el.dataset.taskId);
  const taskName = el.dataset.taskName;
  const projectId = parseInt(el.dataset.projectId) || 0;
  const projectName = el.dataset.projectName;

  closeCommandPalette();

  try {
    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
    navigateTo('timer');
  } catch (err) { showToast(String(err)); }
}

// Global keyboard shortcut
document.addEventListener('keydown', (e) => {
  // Composer first: while it is open it owns Escape/Enter/Tab on its own card
  // listener (which stops propagation), so nothing below may also fire. Escape
  // is still honoured here in case focus escaped the card.
  if (composer.isOpen()) {
    if (e.key === 'Escape') { e.preventDefault(); composer.requestClose(); }
    return;
  }

  // Ctrl/Cmd+Shift+L: add a time entry for the date currently on screen
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === 'L' || e.key === 'l')) {
    e.preventDefault();
    if (store.getState().auth.authenticated) composer.open({ date: getHistoryDate() });
    return;
  }

  // Ctrl+K / Cmd+K to open
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault();
    if (commandPaletteOpen) {
      closeCommandPalette();
    } else if (store.getState().auth.authenticated) {
      openCommandPalette();
    }
    return;
  }

  if (!commandPaletteOpen) return;

  if (e.key === 'Escape') {
    e.preventDefault();
    closeCommandPalette();
    return;
  }

  const items = $$('#command-results .command-item');
  const count = items.length;

  if (e.key === 'ArrowDown') {
    e.preventDefault();
    commandActiveIndex = count > 0 ? (commandActiveIndex + 1) % count : -1;
    updateCommandActive();
    return;
  }

  if (e.key === 'ArrowUp') {
    e.preventDefault();
    commandActiveIndex = count > 0 ? (commandActiveIndex - 1 + count) % count : -1;
    updateCommandActive();
    return;
  }

  if (e.key === 'Enter') {
    e.preventDefault();
    if (commandActiveIndex >= 0 && items[commandActiveIndex]) {
      selectCommandItem(items[commandActiveIndex]);
    }
    return;
  }
});

// Search input
$('#command-search')?.addEventListener('input', (e) => {
  clearTimeout(commandSearchTimeout);
  const q = e.target.value.trim();
  if (q.length === 0) {
    loadCommandRecentTasks();
    return;
  }
  commandSearchTimeout = setTimeout(async () => {
    try {
      const tasks = await api.searchTasks(q, null);
      commandTasks = tasks || [];
      renderCommandResults(commandTasks, 'Search Results');
    } catch (_) {
      renderCommandResults([], '');
    }
  }, 200);
});

// Click on command result item
document.addEventListener('click', (e) => {
  const item = e.target.closest('#command-results .command-item[data-task-id]');
  if (item) {
    selectCommandItem(item);
    return;
  }
});

// Backdrop click closes palette
$('#command-backdrop')?.addEventListener('click', closeCommandPalette);

// ── Filter Chips ─────────────────────────────────────────────────────

let activeFilter = 'all';
let unfilteredTasks = [];

document.addEventListener('click', (e) => {
  const chip = e.target.closest('.filter-chip[data-filter]');
  if (!chip) return;

  activeFilter = chip.dataset.filter;
  $$('.filter-chip').forEach(c => c.classList.toggle('active', c.dataset.filter === activeFilter));

  applyFilter();
});

async function applyFilter() {
  const listEl = $('#task-list');
  if (!listEl) return;

  // Load tasks if we don't have them
  if (unfilteredTasks.length === 0) {
    try {
      if (activeTab === 'my-tasks') {
        unfilteredTasks = await api.getMyTasks();
      } else {
        unfilteredTasks = await api.getRecentTasks();
      }
    } catch (_) { return; }
  }

  let filtered = unfilteredTasks;

  if (activeFilter === 'priority') {
    filtered = unfilteredTasks.filter(t => t.priority === '1' || t.priority === 1 || t.is_priority);
  } else if (activeFilter === 'my-tasks') {
    // Load my tasks specifically
    try {
      filtered = await api.getMyTasks();
    } catch (_) { filtered = unfilteredTasks; }
  } else if (activeFilter === 'overdue') {
    const now = new Date();
    filtered = unfilteredTasks.filter(t => {
      if (!t.date_deadline) return false;
      return new Date(t.date_deadline + 'T23:59:59') < now;
    });
  }
  // else 'all' — show everything

  renderTaskList(listEl, filtered);

  // If in kanban mode, also refresh kanban with filtered tasks
  if (kanbanMode) {
    renderKanbanBoardWithTasks(filtered);
  }
}

function renderKanbanBoardWithTasks(tasks) {
  const kanbanEl = $('#task-kanban');
  if (!kanbanEl) return;

  if (!tasks || tasks.length === 0) {
    kanbanEl.innerHTML = '<div class="empty-state"><p>No tasks match filter</p></div>';
    return;
  }

  const groups = {};
  for (const key of STATE_ORDER) {
    groups[key] = [];
  }
  groups['_other'] = [];

  for (const t of tasks) {
    const stateKey = t.state || '';
    if (groups[stateKey]) {
      groups[stateKey].push(t);
    } else {
      groups['_other'].push(t);
    }
  }

  let html = '';
  for (const key of STATE_ORDER) {
    const cfg = STATE_CONFIG[key];
    const columnTasks = groups[key];
    html += `<div class="kanban-column" data-state="${escAttr(key)}">
      <div class="kanban-column-header">
        <span class="kanban-column-dot" style="background: ${cfg.color}"></span>
        <span class="kanban-column-title">${cfg.label}</span>
        <span class="kanban-column-count">${columnTasks.length}</span>
      </div>
      <div class="kanban-column-body">`;
    if (columnTasks.length === 0) {
      html += '<div class="kanban-empty">No tasks</div>';
    } else {
      for (const t of columnTasks) {
        html += renderTaskCard(t);
      }
    }
    html += '</div></div>';
  }

  if (groups['_other'].length > 0) {
    html += `<div class="kanban-column">
      <div class="kanban-column-header">
        <span class="kanban-column-dot" style="background: var(--text-muted)"></span>
        <span class="kanban-column-title">Other</span>
        <span class="kanban-column-count">${groups['_other'].length}</span>
      </div>
      <div class="kanban-column-body">`;
    for (const t of groups['_other']) {
      html += renderTaskCard(t);
    }
    html += '</div></div>';
  }

  kanbanEl.innerHTML = html;
}

// Reset filter chips when entering tasks view
store.subscribe((state, prev) => {
  if (state.view === 'tasks' && state.view !== prev.view) {
    activeFilter = 'all';
    unfilteredTasks = [];
    $$('.filter-chip').forEach(c => c.classList.toggle('active', c.dataset.filter === 'all'));
  }
});

// ── Quick Switch Bar (timer view one-tap switching) ────────────────────

async function renderQuickSwitchBar() {
  const bar = $('#quick-switch-bar');
  if (!bar) return;

  const timer = store.getState().timer;

  try {
    // Get quick switch entries from backend (respects auto/manual mode)
    const entries = await api.getQuickSwitchEntries();

    if (!entries || entries.length === 0) {
      bar.style.display = 'none';
      return;
    }

    const mainEntries = entries.filter(e => e.slot === 'main');
    const smallEntries = entries.filter(e => e.slot === 'small');

    const mainContainer = $('#qs-bar-main');
    const pillsContainer = $('#qs-bar-pills');

    // Render main items (large, full-width rows)
    mainContainer.innerHTML = mainEntries.map(e => {
      const isActive = timer.is_running && timer.task_id === e.task_id;
      return `<div class="qs-bar-item${isActive ? ' qs-bar-item-active' : ''}"
        data-task-id="${e.task_id}" data-task-name="${escAttr(e.task_name)}"
        data-project-id="${e.project_id}" data-project-name="${escAttr(e.project_name)}">
        <div class="qs-bar-item-info">
          <div class="qs-bar-item-name">${esc(e.task_name)}</div>
          <div class="qs-bar-item-project">${esc(e.project_name)}</div>
        </div>
        <span class="qs-bar-item-arrow">
          ${isActive ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="var(--brand)" stroke="var(--brand)" stroke-width="2"><circle cx="12" cy="12" r="5"/></svg>'
          : '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>'}
        </span>
      </div>`;
    }).join('');

    // Render pill items (compact, side-by-side)
    pillsContainer.innerHTML = smallEntries.map(e => {
      const isActive = timer.is_running && timer.task_id === e.task_id;
      return `<div class="qs-bar-pill${isActive ? ' qs-bar-pill-active' : ''}"
        data-task-id="${e.task_id}" data-task-name="${escAttr(e.task_name)}"
        data-project-id="${e.project_id}" data-project-name="${escAttr(e.project_name)}">
        ${esc(e.task_name)}
      </div>`;
    }).join('');

    bar.style.display = '';
  } catch (err) {
    bar.style.display = 'none';
  }
}

// Click handler for quick switch bar items
document.addEventListener('click', async (e) => {
  const item = e.target.closest('.qs-bar-item[data-task-id], .qs-bar-pill[data-task-id]');
  if (!item) return;

  const taskId = parseInt(item.dataset.taskId);
  if (!taskId) return;
  const taskName = item.dataset.taskName;
  const projectId = parseInt(item.dataset.projectId) || 0;
  const projectName = item.dataset.projectName;

  try {
    const timer = store.getState().timer;
    // Skip if already on this task
    if (timer.is_running && timer.task_id === taskId) return;

    // If timer running on different task, stop & auto-log
    if (timer.is_running) {
      const stopped = await api.stopTimer();
      const hours = stopped.elapsed_secs / 3600;
      await api.logTime(stopped.task_id, stopped.project_id || 0, stopped.task_name, stopped.project_name, stopped.task_name, hours, todayDate());
      showToast(`Logged ${formatTime(stopped.elapsed_secs)} for ${stopped.task_name}`, 'success');
    }

    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
  } catch (err) { showToast(String(err)); }
});

// "All tasks" button opens the full task picker
$('#btn-qs-all')?.addEventListener('click', () => {
  const timer = store.getState().timer;
  showTaskPrompt(timer.is_running ? 'switch' : 'checkin');
});

// Re-render quick switch bar when active task changes (not every second tick)
store.subscribe((state, prev) => {
  if (state.timer?.task_id !== prev.timer?.task_id ||
      state.timer?.is_running !== prev.timer?.is_running) {
    renderQuickSwitchBar();
  }
});

// ── Today's Quick Tasks ──────────────────────────────────────────────

async function renderTodayQuickTasks() {
  const container = $('#today-quick-tasks');
  if (!container) return;

  const entries = store.getState().todayEntries || [];
  if (entries.length === 0) {
    container.innerHTML = '';
    return;
  }

  // Deduplicate by task_id and get the most recent ones
  const seen = new Set();
  const uniqueTasks = [];
  for (const e of entries) {
    const key = e.task_id || e.task_name;
    if (!seen.has(key)) {
      seen.add(key);
      uniqueTasks.push(e);
    }
  }

  const display = uniqueTasks.slice(0, 5);

  let html = '<div class="today-quick-tasks-title">Today\'s Tasks</div>';
  for (const t of display) {
    html += `<div class="quick-task-item"
      data-task-id="${t.task_id || 0}" data-task-name="${escAttr(t.task_name || '')}"
      data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
      <span class="quick-task-dot"></span>
      <span class="quick-task-name">${esc(t.task_name || 'Unknown')}</span>
      <span class="quick-task-hours">${formatHours(t.hours || 0)}</span>
      <button type="button" class="btn-icon ec-row-btn quick-task-add" aria-label="Log time on this task" title="Log time on this task">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14"/><path d="M5 12h14"/></svg>
      </button>
    </div>`;
  }

  container.innerHTML = html;
}

// Click on quick task to switch timer
document.addEventListener('click', async (e) => {
  const item = e.target.closest('.quick-task-item[data-task-id]');
  if (!item) return;

  const taskId = parseInt(item.dataset.taskId);
  if (!taskId) return;
  const taskName = item.dataset.taskName;
  const projectId = parseInt(item.dataset.projectId) || 0;
  const projectName = item.dataset.projectName;

  // The trailing "+" logs time instead of switching the timer — the
  // "I forgot to start the timer for that 20 minutes" path. That is about NOW,
  // so it seeds today rather than whatever day History was last browsed to.
  if (e.target.closest('.quick-task-add')) {
    e.stopPropagation();
    composer.open({ date: todayDate(), taskId, taskName, projectId, projectName });
    return;
  }

  try {
    // If timer is running, stop and auto-log
    const timer = store.getState().timer;
    if (timer.is_running && timer.task_id !== taskId) {
      const stopped = await api.stopTimer();
      const hours = stopped.elapsed_secs / 3600;
      await api.logTime(stopped.task_id, stopped.project_id || 0, stopped.task_name, stopped.project_name, stopped.task_name, hours, todayDate());
      showToast(`Logged ${formatTime(stopped.elapsed_secs)} for ${stopped.task_name}`, 'success');
    }

    await api.startTimer(taskId, taskName, projectId, projectName);
    const timerState = await api.getTimerState();
    store.setState({ timer: timerState, stoppedTimer: null });
  } catch (err) { showToast(String(err)); }
});

// ── Timer Breakdown Chart (wide mode) ────────────────────────────────

const BREAKDOWN_COLORS = [
  '#3b82f6', // blue
  '#8b5cf6', // violet
  '#06b6d4', // cyan
  '#10b981', // emerald
  '#f59e0b', // amber
  '#ef4444', // red
  '#ec4899', // pink
  '#14b8a6', // teal
];
const WORKDAY_HOURS = 8;

function renderTimerBreakdown() {
  const container = $('#timer-breakdown');
  if (!container) return;

  const entries = store.getState().todayEntries || [];
  const timer = store.getState().timer;

  // Aggregate hours by task from logged entries
  const taskMap = new Map();
  for (const e of entries) {
    const key = e.task_id || e.task_name;
    if (taskMap.has(key)) {
      taskMap.get(key).hours += (e.hours || 0);
    } else {
      taskMap.set(key, { name: e.task_name || 'Unknown', project: e.project_name || '', hours: e.hours || 0, isRunning: false });
    }
  }

  // Include currently running timer
  if (timer.is_running && timer.elapsed_secs > 0) {
    const key = timer.task_id || timer.task_name;
    const runningHours = timer.elapsed_secs / 3600;
    if (taskMap.has(key)) {
      taskMap.get(key).hours += runningHours;
      taskMap.get(key).isRunning = true;
    } else {
      taskMap.set(key, { name: timer.task_name || 'Unknown', project: timer.project_name || '', hours: runningHours, isRunning: true });
    }
  }

  if (taskMap.size === 0) {
    container.innerHTML = '';
    return;
  }

  const tasks = [...taskMap.values()].sort((a, b) => b.hours - a.hours);
  const totalHours = tasks.reduce((s, t) => s + t.hours, 0);
  const dayPct = Math.min(100, (totalHours / WORKDAY_HOURS) * 100);

  // Day progress header
  let html = `<div class="breakdown-header">
    <span class="breakdown-title">Today</span>
    <span class="breakdown-total">${formatHours(totalHours)} <span class="breakdown-total-target">/ ${WORKDAY_HOURS}h</span></span>
  </div>`;

  // Stacked day progress bar
  html += '<div class="breakdown-day-track">';
  let offset = 0;
  for (let i = 0; i < tasks.length; i++) {
    const t = tasks[i];
    const segPct = (t.hours / WORKDAY_HOURS) * 100;
    const color = BREAKDOWN_COLORS[i % BREAKDOWN_COLORS.length];
    html += `<div class="breakdown-day-seg${t.isRunning ? ' running' : ''}" style="left:${offset}%;width:${Math.min(segPct, 100 - offset)}%;background:${color}"></div>`;
    offset += segPct;
  }
  html += `</div>
  <span class="breakdown-day-pct">${Math.round(dayPct)}% of ${WORKDAY_HOURS}h day</span>`;

  // Individual task bars
  html += '<div class="breakdown-bars">';
  for (let i = 0; i < tasks.length; i++) {
    const t = tasks[i];
    const barPct = Math.max(2, (t.hours / WORKDAY_HOURS) * 100);
    const pctOfDay = Math.round((t.hours / WORKDAY_HOURS) * 100);
    const color = BREAKDOWN_COLORS[i % BREAKDOWN_COLORS.length];
    html += `<div class="breakdown-row">
      <span class="breakdown-dot" style="background:${color}"></span>
      <span class="breakdown-label" title="${esc(t.name)}">${esc(t.name)}</span>
      <div class="breakdown-track"><div class="breakdown-fill${t.isRunning ? ' running' : ''}" style="width:${barPct}%;background:${color}"></div></div>
      <span class="breakdown-value">${formatHours(t.hours)}</span>
      <span class="breakdown-pct">${pctOfDay}%</span>
    </div>`;
  }
  html += '</div>';
  container.innerHTML = html;
}

// Re-render quick tasks and breakdown when today entries change
let _breakdownTickTimer = null;
store.subscribe((state, prev) => {
  if (state.todayEntries !== prev.todayEntries) {
    renderTodayQuickTasks();
    renderQuickSwitchBar();
    renderTimerBreakdown();
  }
  // Also update breakdown on timer ticks (throttled to ~10s for running timer)
  if (state.timer !== prev.timer && state.timer.is_running && !_breakdownTickTimer) {
    _breakdownTickTimer = setTimeout(() => { _breakdownTickTimer = null; renderTimerBreakdown(); }, 10000);
  }
  // Immediate render when timer starts or stops
  if (state.timer.is_running !== prev.timer?.is_running) {
    renderTimerBreakdown();
  }
});


// ── Weekly Chart ─────────────────────────────────────────────────────

async function renderWeeklyChart() {
  const container = $('#weekly-chart');
  if (!container) return;

  // Get the current history date, find the Monday of its week
  const centerDate = getHistoryDate();
  const d = new Date(centerDate + 'T12:00:00');
  const dayOfWeek = d.getDay(); // 0=Sun, 1=Mon, ...
  const mondayOffset = dayOfWeek === 0 ? -6 : 1 - dayOfWeek;
  const monday = new Date(d);
  monday.setDate(monday.getDate() + mondayOffset);

  const dayNames = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const today = todayDate();

  // Fetch entries for each day of the week
  const weekData = [];
  for (let i = 0; i < 7; i++) {
    const dayDate = new Date(monday);
    dayDate.setDate(dayDate.getDate() + i);
    const dateStr = dayDate.toISOString().slice(0, 10);
    weekData.push({ label: dayNames[i], date: dateStr, hours: 0, isToday: dateStr === today });
  }

  // Try to get data from monthly summary for efficiency,
  // or fall back to individual date queries
  try {
    const year = monday.getFullYear();
    const month = monday.getMonth() + 1;
    const summary = await api.getMonthlySummary(year, month);
    const dayMap = {};
    for (const day of summary.days) {
      dayMap[day.date] = day.total_hours;
    }
    // If the week spans two months, get the other month too
    const lastDay = weekData[6].date;
    const lastMonth = parseInt(lastDay.slice(5, 7));
    if (lastMonth !== month) {
      const summary2 = await api.getMonthlySummary(parseInt(lastDay.slice(0, 4)), lastMonth);
      for (const day of summary2.days) {
        dayMap[day.date] = day.total_hours;
      }
    }
    for (const wd of weekData) {
      wd.hours = dayMap[wd.date] || 0;
    }
  } catch (_) {
    // Fallback: just use today's entries for today
    const entries = store.getState().todayEntries || [];
    const todayHours = entries.reduce((sum, e) => sum + (e.hours || 0), 0);
    for (const wd of weekData) {
      if (wd.date === today) wd.hours = todayHours;
    }
  }

  const maxHours = Math.max(8, ...weekData.map(d => d.hours));

  container.style.display = '';
  let html = '<div class="weekly-chart-title">This Week</div>';
  html += '<div class="weekly-chart-bars">';

  for (const wd of weekData) {
    const pct = maxHours > 0 ? Math.max(3, (wd.hours / maxHours) * 100) : 3;
    const todayCls = wd.isToday ? ' today' : '';
    const hoursLabel = wd.hours > 0 ? formatHours(wd.hours) : '';
    html += `<div class="weekly-bar-col">
      ${hoursLabel ? `<span class="weekly-bar-hours">${hoursLabel}</span>` : ''}
      <div class="weekly-bar${todayCls}" style="height: ${pct}%"></div>
      <span class="weekly-bar-label">${wd.label}</span>
    </div>`;
  }

  html += '</div>';
  container.innerHTML = html;
}

// Render weekly chart when history view is shown
store.subscribe((state, prev) => {
  if (state.view === 'history' && state.view !== prev.view) {
    renderWeeklyChart();
  }
  // Also re-render when history date changes (user navigates days)
  if (state.historyDate !== prev.historyDate && state.view === 'history') {
    renderWeeklyChart();
  }
});

// ── Dashboard Window ──────────────────────────────────────────────────

$('#btn-open-dashboard')?.addEventListener('click', async () => {
  try {
    const { WebviewWindow } = window.__TAURI__.webviewWindow;
    // Check if dashboard window already exists
    const allWindows = await window.__TAURI__.window.getAllWindows();
    const existing = allWindows.find(w => w.label === 'dashboard');
    if (existing) {
      await existing.show();
      await existing.setFocus();
      return;
    }
    const dashboard = new WebviewWindow('dashboard', {
      url: 'dashboard.html',
      title: 'Pointeuse — Dashboard',
      width: 1200,
      height: 800,
      minWidth: 900,
      minHeight: 600,
      center: true,
      decorations: false,
    });
    dashboard.once('tauri://error', (e) => {
      showToast('Failed to open dashboard: ' + String(e.payload));
    });
  } catch (err) {
    showToast('Failed to open dashboard: ' + String(err));
  }
});
