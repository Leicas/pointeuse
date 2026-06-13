// Pointeuse — Dashboard Window
// Standalone JS for the dashboard kanban/detail view

// ── Tauri Invoke ─────────────────────────────────────────────────────

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

// ── State Store (same pattern as main.js) ────────────────────────────

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
  tasks: [],
  filteredTasks: [],
  projects: [],
  todayEntries: [],
  selectedTask: null,
  groupMode: 'state',
  sortMode: 'updated',
  activeFilters: new Set(),
  activeSection: 'my-tasks',
  searchQuery: '',
  loading: true,
  detailLoading: false,
  detailTask: null,
  detailStages: [],
  collapsedColumns: new Set(['1_done', '1_canceled']),
  // Time log state
  timelogDate: null,       // YYYY-MM-DD, null = today
  timelogMode: 'day',      // 'day' | 'week' | 'month'
  timelogEntries: [],      // entries for current date
  timelogMonthly: null,    // { days: [...], total_hours }
  timelogAnalysis: null,   // day analysis result
  // All-tasks section state
  allTasksData: [],            // tasks fetched via get_all_tasks
  allTasksUsers: [],           // [{id, name}] from get_all_users
  allTasksFilterProjects: [],  // selected project ids
  allTasksFilterUsers: [],     // selected user ids
  allTasksLoading: false,
  allTasksUsersLoaded: false,
});

// ── API Layer ─────────────────────────────────────────────────────────

const api = {
  getMyTasks: () => invoke('get_my_tasks'),
  searchTasks: (query, projectId) => invoke('search_tasks', { query, projectId: projectId || null }),
  getProjects: () => invoke('get_projects'),
  getTodayEntries: () => invoke('get_today_entries'),
  getTaskStages: (taskId, projectId) => invoke('get_task_stages', { taskId, projectId }),
  updateTaskStage: (taskId, stageId) => invoke('update_task_stage', { taskId, stageId }),
  updateTaskState: (taskId, newState) => invoke('update_task_state', { taskId, newState }),
  startTimer: (taskId, taskName, projectId, projectName) => invoke('start_timer', { taskId, taskName, projectId, projectName }),
  createTask: (name, projectId) => invoke('create_task', { name, projectId }),
  logTime: (taskId, projectId, taskName, projectName, description, durationHours, date) =>
    invoke('log_time', { taskId, projectId, taskName, projectName, description, durationHours, date }),
  getRecentTasks: () => invoke('get_recent_tasks'),
  checkAuth: () => invoke('check_auth'),
  updateTaskName: (taskId, name) => invoke('update_task_name', { taskId, name }),
  updateTaskDescription: (taskId, description) => invoke('update_task_description', { taskId, description }),
  updateTaskDeadline: (taskId, dateDeadline) => invoke('update_task_deadline', { taskId, dateDeadline }),
  updateTaskPriority: (taskId, priority) => invoke('update_task_priority', { taskId, priority }),
  getTaskDetails: (taskId) => invoke('get_task_details', { taskId }),
  getSavedConnection: () => invoke('get_saved_connection'),
  getEntriesForDate: (date) => invoke('get_entries_for_date', { date }),
  getMonthlySummary: (year, month) => invoke('get_monthly_summary', { year, month }),
  getDayAnalysis: (date) => invoke('get_day_analysis', { date }),
  getAllTasks: (projectIds, userIds) => invoke('get_all_tasks', { projectIds: projectIds || [], userIds: userIds || [] }),
  getAllUsers: () => invoke('get_all_users'),
};

// Odoo URL cache for "Open in Odoo" links
let cachedOdooUrl = '';
async function getOdooUrl() {
  if (cachedOdooUrl) return cachedOdooUrl;
  try {
    const conn = await api.getSavedConnection();
    cachedOdooUrl = (conn?.url || '').replace(/\/$/, '');
    return cachedOdooUrl;
  } catch (_) { return ''; }
}

// ── Helpers ───────────────────────────────────────────────────────────

function $(sel) { return document.querySelector(sel); }
function $$(sel) { return document.querySelectorAll(sel); }

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

function todayDate() {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function formatHours(hours) {
  const abs = Math.abs(hours);
  const sign = hours < 0 ? '-' : '';
  const h = Math.floor(abs);
  const m = Math.round((abs - h) * 60);
  return `${sign}${h}h ${m}m`;
}

function formatTime(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

function debounce(fn, ms) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

// ── State Config ─────────────────────────────────────────────────────

const STATE_CONFIG = {
  '01_in_progress':       { label: 'In Progress',       color: '#3b82f6' },
  '02_changes_requested': { label: 'Changes Requested', color: '#ef4444' },
  '03_approved':          { label: 'Approved',          color: '#a855f7' },
  '04_waiting_normal':    { label: 'Waiting',           color: '#f59e0b' },
  '1_done':               { label: 'Done',              color: '#22c55e' },
  '1_canceled':           { label: 'Canceled',          color: '#6b7280' },
};

const STATE_ORDER = [
  '01_in_progress',
  '02_changes_requested',
  '03_approved',
  '04_waiting_normal',
  '1_done',
  '1_canceled',
];

// ── Toast ─────────────────────────────────────────────────────────────

let toastTimeout;
function showToast(msg, type = 'error') {
  const el = $('#dashboard-toast');
  if (!el) return;
  el.textContent = msg;
  el.className = `toast visible toast-${type}`;
  clearTimeout(toastTimeout);
  toastTimeout = setTimeout(() => el.classList.remove('visible'), 4000);
}

// ── Titlebar ──────────────────────────────────────────────────────────

$('.titlebar')?.addEventListener('mousedown', async (e) => {
  if (e.target.closest('.titlebar-btn')) return;
  if (e.buttons === 1) {
    try { await window.__TAURI__.window.getCurrentWindow().startDragging(); } catch (_) {}
  }
});

$('#titlebar-minimize')?.addEventListener('click', async () => {
  try { await window.__TAURI__.window.getCurrentWindow().minimize(); } catch (_) {}
});
$('#titlebar-close')?.addEventListener('click', async () => {
  try { await window.__TAURI__.window.getCurrentWindow().close(); } catch (_) {}
});

// ── Data Loading ─────────────────────────────────────────────────────

async function loadAllData() {
  store.setState({ loading: true });
  // Preload Odoo URL for "Open in Odoo" links
  getOdooUrl();
  try {
    const [tasks, projects, todayEntries] = await Promise.all([
      api.getMyTasks(),
      api.getProjects(),
      api.getTodayEntries(),
    ]);
    store.setState({
      tasks: tasks || [],
      projects: projects || [],
      todayEntries: todayEntries || [],
      loading: false,
    });
    applyFiltersAndRender();
  } catch (err) {
    store.setState({ loading: false });
    showToast('Failed to load data: ' + String(err));
  }
}

async function refreshTasks() {
  try {
    const tasks = await api.getMyTasks();
    store.setState({ tasks: tasks || [] });
    applyFiltersAndRender();
    showToast('Tasks refreshed', 'success');
  } catch (err) {
    showToast('Refresh failed: ' + String(err));
  }
}

async function refreshTodayEntries() {
  try {
    const entries = await api.getTodayEntries();
    store.setState({ todayEntries: entries || [] });
    updateSidebarCounts();
    // Only render time log if we're on that section
    if (store.getState().activeSection === 'timelog') renderTimeLog();
    // If detail panel is open, refresh its time entries
    const { selectedTask } = store.getState();
    if (selectedTask) renderDetailTimeEntries();
  } catch (_) {}
}

// ── Sidebar Navigation ───────────────────────────────────────────────

document.addEventListener('click', (e) => {
  const item = e.target.closest('.sidebar-item[data-section]');
  if (!item) return;
  const section = item.dataset.section;
  $$('.sidebar-item').forEach(el => el.classList.toggle('active', el.dataset.section === section));
  store.setState({ activeSection: section });
  if (section === 'timelog') {
    goToTimelogDate(getTimelogDate());
  } else if (section === 'all-tasks') {
    loadAllTasksSection();
  } else {
    applyFiltersAndRender();
  }
});

// ── All Tasks Section ────────────────────────────────────────────────

// Persist filter selections in localStorage
function saveAllTasksFilters() {
  const { allTasksFilterProjects, allTasksFilterUsers } = store.getState();
  try {
    localStorage.setItem('atf_projects', JSON.stringify(allTasksFilterProjects));
    localStorage.setItem('atf_users', JSON.stringify(allTasksFilterUsers));
  } catch (_) {}
}
function loadAllTasksFilters() {
  try {
    const p = JSON.parse(localStorage.getItem('atf_projects') || '[]');
    const u = JSON.parse(localStorage.getItem('atf_users') || '[]');
    if (Array.isArray(p) && Array.isArray(u)) {
      store.setState({ allTasksFilterProjects: p, allTasksFilterUsers: u });
    }
  } catch (_) {}
}
// Restore on load
loadAllTasksFilters();

async function loadAllTasksSection() {
  const { allTasksUsersLoaded } = store.getState();

  // Load users list once
  if (!allTasksUsersLoaded) {
    api.getAllUsers().then(users => {
      store.setState({ allTasksUsers: users || [], allTasksUsersLoaded: true });
      if (store.getState().activeSection === 'all-tasks') renderAllTasksFilterBar();
    }).catch(() => {});
  }

  await fetchAllTasks();
}

async function fetchAllTasks() {
  // Always fetch ALL tasks (no backend filter) so cache works with any filter combo
  store.setState({ allTasksLoading: true });
  applyFiltersAndRender();

  try {
    const tasks = await api.getAllTasks([], []);
    store.setState({ allTasksData: tasks || [], allTasksLoading: false });
    applyFiltersAndRender();
  } catch (err) {
    store.setState({ allTasksLoading: false });
    showToast('Failed to load all tasks: ' + String(err));
    applyFiltersAndRender();
  }
}

function renderAllTasksFilterBar() {
  const bar = $('#all-tasks-filter-bar');
  if (!bar) return;

  const { projects, allTasksUsers, allTasksFilterProjects, allTasksFilterUsers } = store.getState();
  const hasFilters = allTasksFilterProjects.length > 0 || allTasksFilterUsers.length > 0;

  // Build summary text for each dropdown
  const projLabel = allTasksFilterProjects.length === 0
    ? 'All projects'
    : allTasksFilterProjects.length === 1
      ? (projects.find(p => p.id === allTasksFilterProjects[0])?.name || '1 project')
      : `${allTasksFilterProjects.length} projects`;
  const userLabel = allTasksFilterUsers.length === 0
    ? 'All assignees'
    : allTasksFilterUsers.length === 1
      ? (allTasksUsers.find(u => u.id === allTasksFilterUsers[0])?.name || '1 assignee')
      : `${allTasksFilterUsers.length} assignees`;

  let html = `
    <div class="atf-dropdown-group">
      <button class="atf-dropdown-btn" id="atf-proj-btn">
        <span class="atf-dropdown-label">Project</span>
        <span class="atf-dropdown-value${allTasksFilterProjects.length > 0 ? ' has-filter' : ''}">${esc(projLabel)}</span>
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
      </button>
      <div class="atf-dropdown-panel" id="atf-proj-panel">
        <input class="atf-search" id="atf-proj-search" placeholder="Search projects\u2026" autocomplete="off" />
        <div class="atf-options" id="atf-proj-options">
          ${projects.map(p => {
            const active = allTasksFilterProjects.includes(p.id);
            return `<label class="atf-option${active ? ' active' : ''}">
              <input type="checkbox" ${active ? 'checked' : ''} data-atf-project="${p.id}" />
              <span>${esc(p.name)}</span>
            </label>`;
          }).join('')}
        </div>
      </div>
    </div>
    <div class="atf-dropdown-group">
      <button class="atf-dropdown-btn" id="atf-user-btn">
        <span class="atf-dropdown-label">Assignee</span>
        <span class="atf-dropdown-value${allTasksFilterUsers.length > 0 ? ' has-filter' : ''}">${esc(userLabel)}</span>
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="6 9 12 15 18 9"/></svg>
      </button>
      <div class="atf-dropdown-panel" id="atf-user-panel">
        <input class="atf-search" id="atf-user-search" placeholder="Search people\u2026" autocomplete="off" />
        <div class="atf-options" id="atf-user-options">
          ${allTasksUsers.map(u => {
            const active = allTasksFilterUsers.includes(u.id);
            return `<label class="atf-option${active ? ' active' : ''}">
              <input type="checkbox" ${active ? 'checked' : ''} data-atf-user="${u.id}" />
              <span>${esc(u.name)}</span>
            </label>`;
          }).join('')}
        </div>
      </div>
    </div>
    ${hasFilters ? '<button class="atf-clear" id="atf-clear-all">Clear</button>' : ''}
    <div style="flex:1"></div>
    <span class="atf-count" id="atf-result-count"></span>`;

  bar.innerHTML = html;

  // Update result count
  updateFilterCount();
}

function updateFilterCount() {
  const el = document.getElementById('atf-result-count');
  if (!el) return;
  const { filteredTasks, allTasksData } = store.getState();
  const total = allTasksData.length;
  const shown = filteredTasks.length;
  el.textContent = shown === total ? `${total} tasks` : `${shown} of ${total} tasks`;
}

// Dropdown toggle logic
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.atf-dropdown-btn');
  if (btn) {
    const panel = btn.nextElementSibling;
    const wasOpen = panel.classList.contains('open');
    // Close all panels first
    $$('.atf-dropdown-panel.open').forEach(p => p.classList.remove('open'));
    if (!wasOpen) {
      panel.classList.add('open');
      const search = panel.querySelector('.atf-search');
      if (search) { search.value = ''; search.focus(); filterDropdownOptions(search); }
    }
    return;
  }
  // Close panels when clicking outside
  if (!e.target.closest('.atf-dropdown-panel') && !e.target.closest('.atf-dropdown-btn')) {
    $$('.atf-dropdown-panel.open').forEach(p => p.classList.remove('open'));
  }
});

// Search within dropdowns
document.addEventListener('input', (e) => {
  if (e.target.classList.contains('atf-search')) {
    filterDropdownOptions(e.target);
  }
});

function filterDropdownOptions(searchEl) {
  const q = searchEl.value.toLowerCase();
  const options = searchEl.parentElement.querySelectorAll('.atf-option');
  options.forEach(opt => {
    const text = opt.querySelector('span').textContent.toLowerCase();
    opt.style.display = text.includes(q) ? '' : 'none';
  });
}

// Checkbox change handler for filter selections
document.addEventListener('change', (e) => {
  const projCb = e.target.closest('[data-atf-project]');
  if (projCb) {
    const pid = parseInt(projCb.dataset.atfProject);
    const { allTasksFilterProjects } = store.getState();
    const newFilter = projCb.checked
      ? [...allTasksFilterProjects, pid]
      : allTasksFilterProjects.filter(id => id !== pid);
    store.setState({ allTasksFilterProjects: newFilter });
    saveAllTasksFilters();
    projCb.closest('.atf-option').classList.toggle('active', projCb.checked);
    applyFiltersAndRender();
    return;
  }

  const userCb = e.target.closest('[data-atf-user]');
  if (userCb) {
    const uid = parseInt(userCb.dataset.atfUser);
    const { allTasksFilterUsers } = store.getState();
    const newFilter = userCb.checked
      ? [...allTasksFilterUsers, uid]
      : allTasksFilterUsers.filter(id => id !== uid);
    store.setState({ allTasksFilterUsers: newFilter });
    saveAllTasksFilters();
    userCb.closest('.atf-option').classList.toggle('active', userCb.checked);
    applyFiltersAndRender();
    return;
  }
});

// Clear all filters
document.addEventListener('click', (e) => {
  if (e.target.closest('#atf-clear-all')) {
    store.setState({ allTasksFilterProjects: [], allTasksFilterUsers: [] });
    saveAllTasksFilters();
    applyFiltersAndRender();
    return;
  }
});

// ── Sorting ──────────────────────────────────────────────────────────

$('#sort-select')?.addEventListener('change', (e) => {
  store.setState({ sortMode: e.target.value });
  applyFiltersAndRender();
});

// ── Filter Chips ─────────────────────────────────────────────────────

document.addEventListener('click', (e) => {
  const chip = e.target.closest('.filter-chip[data-filter]');
  if (!chip) return;
  const filter = chip.dataset.filter;
  const { activeFilters } = store.getState();
  const newFilters = new Set(activeFilters);
  if (newFilters.has(filter)) {
    newFilters.delete(filter);
  } else {
    newFilters.add(filter);
  }
  store.setState({ activeFilters: newFilters });
  // Update chip visuals
  $$('.filter-chip').forEach(c => c.classList.toggle('active', newFilters.has(c.dataset.filter)));
  applyFiltersAndRender();
});

// Empty-state actions
document.addEventListener('click', (e) => {
  if (e.target.closest('#dash-clear-filters')) {
    store.setState({ activeFilters: new Set(), searchQuery: '' });
    $$('.filter-chip').forEach(c => c.classList.remove('active'));
    const search = $('#dashboard-search');
    if (search) search.value = '';
    applyFiltersAndRender();
    return;
  }
  if (e.target.closest('#dash-empty-new-task')) {
    toggleNewTaskForm();
  }
});

// ── Search ───────────────────────────────────────────────────────────

let searchTimeout;
$('#dashboard-search')?.addEventListener('input', (e) => {
  clearTimeout(searchTimeout);
  const q = e.target.value.trim();
  store.setState({ searchQuery: q });
  if (q.length > 0) {
    searchTimeout = setTimeout(async () => {
      try {
        const tasks = await api.searchTasks(q, null);
        if (store.getState().activeSection === 'all-tasks') {
          store.setState({ allTasksData: tasks || [] });
        } else {
          store.setState({ tasks: tasks || [] });
        }
        applyFiltersAndRender();
      } catch (_) {}
    }, 300);
  } else {
    // Reload original tasks
    if (store.getState().activeSection === 'all-tasks') {
      fetchAllTasks();
    } else {
      (async () => {
        try {
          const tasks = await api.getMyTasks();
          store.setState({ tasks: tasks || [] });
          applyFiltersAndRender();
        } catch (_) {}
      })();
    }
  }
});

// ── Filtering + Sorting Logic ────────────────────────────────────────

function applyFiltersAndRender() {
  const { tasks, activeSection, activeFilters, sortMode, allTasksData, allTasksLoading } = store.getState();

  // All-tasks section uses its own data source with client-side filtering
  if (activeSection === 'all-tasks') {
    const { allTasksFilterProjects, allTasksFilterUsers } = store.getState();
    let filtered = [...allTasksData];

    // Project filter (client-side)
    if (allTasksFilterProjects.length > 0) {
      filtered = filtered.filter(t => t.project_id && allTasksFilterProjects.includes(t.project_id));
    }
    // User/assignee filter (client-side)
    if (allTasksFilterUsers.length > 0) {
      filtered = filtered.filter(t => {
        const uids = t.user_ids || [];
        return uids.some(uid => allTasksFilterUsers.includes(uid));
      });
    }

    // Toggle filters still apply
    if (activeFilters.has('priority')) {
      filtered = filtered.filter(t => t.priority === '1' || t.priority === 1 || t.is_priority);
    }
    if (activeFilters.has('overdue')) {
      const now = new Date();
      filtered = filtered.filter(t => {
        if (!t.date_deadline) return false;
        return new Date(t.date_deadline + 'T23:59:59') < now;
      });
    }
    if (activeFilters.has('stale')) {
      const sevenDaysAgo = new Date();
      sevenDaysAgo.setDate(sevenDaysAgo.getDate() - 7);
      filtered = filtered.filter(t => {
        if (!t.write_date) return false;
        return new Date(t.write_date) < sevenDaysAgo;
      });
    }

    filtered = sortTasks(filtered, sortMode);
    store.setState({ filteredTasks: filtered });
    renderMainArea();
    updateFilterCount();
    return;
  }

  let filtered = [...tasks];

  // Section filter
  switch (activeSection) {
    case 'my-tasks':
      filtered = filtered.filter(t => t.state !== '1_done' && t.state !== '1_canceled');
      break;
    case 'today': {
      const today = todayDate();
      filtered = filtered.filter(t => {
        if (!t.write_date) return false;
        return t.write_date.startsWith(today);
      });
      break;
    }
    case 'done':
      filtered = filtered.filter(t => t.state === '1_done');
      break;
    case 'timelog':
    case 'time-log':
      renderTimeLog();
      return;
    default:
      break;
  }

  // Toggle filters
  if (activeFilters.has('priority')) {
    filtered = filtered.filter(t => t.priority === '1' || t.priority === 1 || t.is_priority);
  }
  if (activeFilters.has('overdue')) {
    const now = new Date();
    filtered = filtered.filter(t => {
      if (!t.date_deadline) return false;
      return new Date(t.date_deadline + 'T23:59:59') < now;
    });
  }
  if (activeFilters.has('stale')) {
    const sevenDaysAgo = new Date();
    sevenDaysAgo.setDate(sevenDaysAgo.getDate() - 7);
    filtered = filtered.filter(t => {
      if (!t.write_date) return false;
      return new Date(t.write_date) < sevenDaysAgo;
    });
  }

  // Sort
  filtered = sortTasks(filtered, sortMode);

  store.setState({ filteredTasks: filtered });
  renderMainArea();
}

function sortTasks(tasks, mode) {
  const sorted = [...tasks];
  switch (mode) {
    case 'deadline':
      sorted.sort((a, b) => {
        if (!a.date_deadline && !b.date_deadline) return 0;
        if (!a.date_deadline) return 1;
        if (!b.date_deadline) return -1;
        return a.date_deadline.localeCompare(b.date_deadline);
      });
      break;
    case 'updated':
      sorted.sort((a, b) => {
        if (!a.write_date && !b.write_date) return 0;
        if (!a.write_date) return 1;
        if (!b.write_date) return -1;
        return b.write_date.localeCompare(a.write_date);
      });
      break;
    case 'priority':
      sorted.sort((a, b) => {
        const ap = (a.priority === '1' || a.priority === 1 || a.is_priority) ? 0 : 1;
        const bp = (b.priority === '1' || b.priority === 1 || b.is_priority) ? 0 : 1;
        return ap - bp;
      });
      break;
    case 'name':
      sorted.sort((a, b) => (a.name || '').localeCompare(b.name || ''));
      break;
  }
  return sorted;
}

// ── Main Area Rendering ──────────────────────────────────────────────

function renderMainArea() {
  const { activeSection } = store.getState();
  const mainEl = $('#main-area');
  if (!mainEl) return;

  if (activeSection === 'timelog') {
    renderTimeLog();
    return;
  }

  // Inject or remove the all-tasks filter bar
  if (activeSection === 'all-tasks') {
    if (!$('#all-tasks-filter-bar')) {
      const bar = document.createElement('div');
      bar.id = 'all-tasks-filter-bar';
      bar.className = 'all-tasks-filter-bar';
      mainEl.parentNode.insertBefore(bar, mainEl);
    }
    renderAllTasksFilterBar();
  } else {
    const existingBar = $('#all-tasks-filter-bar');
    if (existingBar) existingBar.remove();
  }

  const { groupMode, allTasksLoading } = store.getState();

  // Show loading state for all-tasks
  if (activeSection === 'all-tasks' && allTasksLoading) {
    mainEl.innerHTML = '<div class="dash-loading"><div class="dash-spinner"></div><p>Loading all tasks...</p></div>';
    updateSidebarCounts();
    return;
  }

  if (groupMode === 'project') {
    renderProjectBoard();
  } else {
    renderKanbanBoard();
  }
  updateSidebarCounts();
}

// ── Sidebar Counts ──────────────────────────────────────────────────

function updateSidebarCounts() {
  const { tasks, todayEntries, allTasksData } = store.getState();
  const today = todayDate();
  const myCount = tasks.filter(t => t.state !== '1_done' && t.state !== '1_canceled').length;
  const todayCount = tasks.filter(t => t.write_date && t.write_date.startsWith(today)).length;
  const allCount = allTasksData.length > 0 ? allTasksData.length : tasks.length;
  const doneCount = tasks.filter(t => t.state === '1_done').length;

  const setCount = (id, n) => { const el = document.getElementById(id); if (el) el.textContent = n; };
  setCount('count-my-tasks', myCount);
  setCount('count-today', todayCount);
  setCount('count-all-tasks', allCount);
  setCount('count-done', doneCount);
  // Time log count = today entries
  const timelogCountEl = document.querySelector('[data-section="timelog"] .sidebar-count');
  if (timelogCountEl) timelogCountEl.textContent = todayEntries.length;
}

// ── Group Toggle ────────────────────────────────────────────────────

document.addEventListener('click', (e) => {
  const btn = e.target.closest('#group-toggle .toggle-btn');
  if (!btn) return;
  const mode = btn.dataset.group;
  if (!mode) return;
  store.setState({ groupMode: mode });
  $$('#group-toggle .toggle-btn').forEach(b => b.classList.toggle('active', b.dataset.group === mode));
  applyFiltersAndRender();
});

// ── Project Board ───────────────────────────────────────────────────

function renderProjectBoard() {
  const mainEl = $('#main-area');
  if (!mainEl) return;

  const { filteredTasks, loading, collapsedColumns } = store.getState();

  if (loading) {
    mainEl.innerHTML = '<div class="dash-loading"><div class="dash-spinner"></div><p>Loading tasks...</p></div>';
    return;
  }

  if (!filteredTasks || filteredTasks.length === 0) {
    mainEl.innerHTML = '<div class="dash-empty"><p>No tasks match your filters</p></div>';
    return;
  }

  // Group by project
  const groups = {};
  for (const t of filteredTasks) {
    const projName = t.project_name || 'No Project';
    if (!groups[projName]) groups[projName] = [];
    groups[projName].push(t);
  }

  const projectNames = Object.keys(groups).sort((a, b) => {
    if (a === 'No Project') return 1;
    if (b === 'No Project') return -1;
    return a.localeCompare(b);
  });

  let html = '<div id="kanban-board" style="display:flex">';
  for (const projName of projectNames) {
    const projectTasks = groups[projName];
    const isCollapsed = collapsedColumns.has('proj_' + projName);
    // Pick a color based on project name hash
    const colors = ['#3b82f6', '#a855f7', '#22c55e', '#f59e0b', '#ef4444', '#06b6d4', '#ec4899', '#84cc16'];
    let hash = 0;
    for (let i = 0; i < projName.length; i++) hash = ((hash << 5) - hash + projName.charCodeAt(i)) | 0;
    const color = colors[Math.abs(hash) % colors.length];

    html += `<div class="kanban-column${isCollapsed ? ' collapsed' : ''}" data-state="proj_${escAttr(projName)}">
      <div class="kanban-header" data-collapse-state="proj_${escAttr(projName)}">
        <span class="kanban-state-dot" style="background: ${color}"></span>
        <span class="kanban-column-title">${esc(projName)}</span>
        <span class="kanban-count">${projectTasks.length}</span>
        <button class="kanban-collapse-btn" title="${isCollapsed ? 'Expand' : 'Collapse'}">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <polyline points="${isCollapsed ? '9 18 15 12 9 6' : '18 15 12 9 6 15'}"/>
          </svg>
        </button>
      </div>
      <div class="kanban-body">`;

    if (projectTasks.length === 0) {
      html += '<div class="kanban-empty">No tasks</div>';
    } else {
      for (const t of projectTasks) {
        html += renderTaskCard(t);
      }
    }
    html += '</div></div>';
  }

  html += '</div>';
  mainEl.innerHTML = html;
}

// ── Kanban Board ─────────────────────────────────────────────────────

function getDeadlinePill(task) {
  if (!task.date_deadline) return '';
  const now = new Date();
  const deadline = new Date(task.date_deadline + 'T23:59:59');
  const diffDays = Math.ceil((deadline - now) / 86400000);

  let text, cls, glyph, label;
  if (diffDays < 0)       { text = `${Math.abs(diffDays)}d overdue`; cls = 'overdue';  glyph = '⚠'; label = `Overdue by ${Math.abs(diffDays)} days`; }
  else if (diffDays === 0){ text = 'Due today';                     cls = 'due-soon'; glyph = '●'; label = 'Due today'; }
  else if (diffDays <= 3) { text = `${diffDays}d left`;             cls = 'due-soon'; glyph = '◐'; label = `Due in ${diffDays} days`; }
  else                    { text = `${diffDays}d left`;             cls = '';         glyph = '○'; label = `Due in ${diffDays} days`; }

  return `<span class="task-card-deadline ${cls}" aria-label="${escAttr(label)}" title="${escAttr(label)}"><span class="tcd-glyph" aria-hidden="true">${glyph}</span>${text}</span>`;
}

function renderTaskCard(task) {
  const stateKey = task.state || '';
  const cfg = STATE_CONFIG[stateKey] || { label: '', color: 'var(--brand)' };
  const deadlinePill = getDeadlinePill(task);
  const projectName = task.project_name || 'No project';
  const isPriority = task.priority === '1' || task.priority === 1 || task.is_priority;
  const priorityStar = isPriority
    ? '<span class="task-card-priority">&#9733;</span>'
    : '<span class="task-card-priority low">&#9734;</span>';

  const planned = Number(task.planned_hours) || 0;
  const effective = Number(task.effective_hours) || 0;
  let progress = '';
  if (planned > 0) {
    const pct = Math.min(100, Math.round((effective / planned) * 100));
    progress = `<div class="task-card-progress" title="${effective.toFixed(1)}h / ${planned.toFixed(1)}h logged"><i style="width:${pct}%"></i></div>`;
  }

  return `<div class="task-card${isPriority ? ' is-priority' : ''}" tabindex="0" role="button"
    data-task-id="${task.id}" data-task-name="${escAttr(task.name)}"
    data-project-id="${task.project_id || 0}" data-project-name="${escAttr(task.project_name || '')}"
    data-state="${escAttr(stateKey)}">
    <div class="task-card-top">
      <div class="task-card-name">${esc(task.name)}</div>
      <button class="task-card-play" data-tc-play title="Start timer" aria-label="Start timer on this task">
        <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><polygon points="7 4 20 12 7 20"/></svg>
      </button>
    </div>
    <div class="task-card-meta">
      <span class="task-card-state-dot" style="background: ${cfg.color}"></span>
      <span class="task-card-project">${esc(projectName)}</span>
      ${deadlinePill}
      ${priorityStar}
    </div>
    ${progress}
  </div>`;
}

function renderKanbanBoard() {
  const mainEl = $('#main-area');
  if (!mainEl) return;

  const { filteredTasks, loading, collapsedColumns } = store.getState();

  if (loading) {
    mainEl.innerHTML = '<div class="dash-loading"><div class="dash-spinner"></div><p>Loading tasks...</p></div>';
    return;
  }

  if (!filteredTasks || filteredTasks.length === 0) {
    const { activeFilters, searchQuery, tasks } = store.getState();
    const isFiltered = (activeFilters && activeFilters.size) || (searchQuery && searchQuery.trim());
    mainEl.innerHTML = isFiltered
      ? `<div class="dash-empty"><p>No tasks match your filters</p>
           <button class="btn btn-secondary btn-sm" id="dash-clear-filters">Clear filters</button></div>`
      : (tasks && tasks.length)
        ? `<div class="dash-empty"><p>All caught up — nothing in this view.</p></div>`
        : `<div class="dash-empty"><p>No tasks yet</p>
             <button class="btn btn-primary btn-sm" id="dash-empty-new-task">Create your first task</button></div>`;
    return;
  }

  // Totals from unfiltered tasks, for "shown / total" labels when filtering
  const { tasks: allTasks, activeFilters: af, searchQuery: sq } = store.getState();
  const isFiltered = (af && af.size) || (sq && sq.trim());
  const totalByState = {};
  for (const t of (allTasks || [])) {
    const k = (t.state && (STATE_CONFIG[t.state] ? t.state : '_other')) || '_other';
    totalByState[k] = (totalByState[k] || 0) + 1;
  }

  // Group by state
  const groups = {};
  for (const key of STATE_ORDER) {
    groups[key] = [];
  }
  groups['_other'] = [];

  for (const t of filteredTasks) {
    const stateKey = t.state || '';
    if (groups[stateKey]) {
      groups[stateKey].push(t);
    } else {
      groups['_other'].push(t);
    }
  }

  let html = '<div id="kanban-board" style="display:flex">';
  for (const key of STATE_ORDER) {
    const cfg = STATE_CONFIG[key];
    const columnTasks = groups[key];
    const isCollapsed = collapsedColumns.has(key);

    html += `<div class="kanban-column${isCollapsed ? ' collapsed' : ''}" data-state="${escAttr(key)}">
      <div class="kanban-header" data-collapse-state="${escAttr(key)}">
        <span class="kanban-state-dot" style="background: ${cfg.color}"></span>
        <span class="kanban-column-title">${cfg.label}</span>
        <span class="kanban-count" title="${columnTasks.length} shown of ${totalByState[key] || 0}">${isFiltered ? `${columnTasks.length} / ${totalByState[key] || 0}` : `${columnTasks.length}`}</span>
        <button class="kanban-collapse-btn" title="${isCollapsed ? 'Expand' : 'Collapse'}">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
            <polyline points="${isCollapsed ? '9 18 15 12 9 6' : '18 15 12 9 6 15'}"/>
          </svg>
        </button>
      </div>
      <div class="kanban-body">`;

    if (columnTasks.length === 0) {
      html += isFiltered ? '<div class="kanban-empty muted">No matches</div>' : '<div class="kanban-empty">No tasks</div>';
    } else {
      for (const t of columnTasks) {
        html += renderTaskCard(t);
      }
    }
    html += '</div></div>';
  }

  // Other column
  if (groups['_other'].length > 0) {
    html += `<div class="kanban-column" data-state="_other">
      <div class="kanban-header">
        <span class="kanban-state-dot" style="background: var(--text-muted)"></span>
        <span class="kanban-column-title">Other</span>
        <span class="kanban-count">${groups['_other'].length}</span>
      </div>
      <div class="kanban-body">`;
    for (const t of groups['_other']) {
      html += renderTaskCard(t);
    }
    html += '</div></div>';
  }

  html += '</div>';
  mainEl.innerHTML = html;
}

// Column collapse toggle
document.addEventListener('click', (e) => {
  const header = e.target.closest('.kanban-header[data-collapse-state]');
  if (!header) return;
  const stateKey = header.dataset.collapseState;
  const { collapsedColumns } = store.getState();
  const newCollapsed = new Set(collapsedColumns);
  if (newCollapsed.has(stateKey)) {
    newCollapsed.delete(stateKey);
  } else {
    newCollapsed.add(stateKey);
  }
  store.setState({ collapsedColumns: newCollapsed });
  renderKanbanBoard();
});

// ── Task Card Click -> Detail Panel ──────────────────────────────────

document.addEventListener('click', async (e) => {
  // Play button: start the timer directly, don't open the detail panel
  const play = e.target.closest('[data-tc-play]');
  if (play) {
    e.stopPropagation();
    const card = play.closest('.task-card[data-task-id]');
    if (!card) return;
    try {
      await api.startTimer(Number(card.dataset.taskId), card.dataset.taskName,
        Number(card.dataset.projectId) || 0, card.dataset.projectName);
      showToast(`Timer started: ${card.dataset.taskName}`, 'success');
    } catch (err) { showToast(prettifyOdooError(err)); }
    return;
  }
  const card = e.target.closest('.task-card[data-task-id]');
  if (!card) return;
  // Skip if inside command palette
  if (card.closest('#cmd-palette')) return;
  const taskId = parseInt(card.dataset.taskId);
  openDetailPanel(taskId);
});

// Enter-to-open for keyboard-focused cards
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter') return;
  const active = document.activeElement;
  if (active && active.matches && active.matches('.task-card[data-task-id]') && !active.closest('#cmd-palette')) {
    e.preventDefault();
    openDetailPanel(parseInt(active.dataset.taskId));
  }
});

// ── Detail Panel ─────────────────────────────────────────────────────

async function openDetailPanel(taskId) {
  const { tasks, allTasksData, activeSection } = store.getState();
  const pool = activeSection === 'all-tasks' ? allTasksData : tasks;
  const task = pool.find(t => t.id === taskId) || tasks.find(t => t.id === taskId);
  if (!task) {
    showToast('Task not found');
    return;
  }

  store.setState({ selectedTask: task, detailLoading: true, detailTask: task, detailStages: [] });
  renderDetailPanel();
  showDetailPanel();

  // Load stages
  try {
    const info = await api.getTaskStages(taskId, task.project_id || 0);
    store.setState({
      detailLoading: false,
      detailTask: { ...task, _stageId: info.stage_id, _state: info.state || task.state },
      detailStages: info.available_stages || [],
    });
    renderDetailPanel();
  } catch (_) {
    store.setState({ detailLoading: false });
    renderDetailPanel();
  }
}

function showDetailPanel() {
  const panel = $('#detail-panel');
  if (panel) {
    panel.classList.add('open');
    // Trap focus
    setTimeout(() => {
      const firstInput = panel.querySelector('input, select, textarea, button');
      if (firstInput) firstInput.focus();
    }, 100);
  }
}

function closeDetailPanel() {
  const panel = $('#detail-panel');
  if (panel) panel.classList.remove('open');
  store.setState({ selectedTask: null, detailTask: null, detailStages: [] });
}

function renderDetailPanel() {
  const panel = $('#detail-panel');
  if (!panel) return;

  const { detailTask, detailStages, detailLoading, todayEntries } = store.getState();
  if (!detailTask) {
    panel.innerHTML = '';
    return;
  }

  const task = detailTask;
  const stateKey = task._state || task.state || '';
  const cfg = STATE_CONFIG[stateKey] || { label: 'Unknown', color: 'var(--brand)' };
  const isPriority = task.priority === '1' || task.priority === 1 || task.is_priority;
  const deadlinePill = getDeadlinePill(task);

  // Task time entries for today
  const taskEntries = (todayEntries || []).filter(e => e.task_id === task.id);
  const taskTotalHours = taskEntries.reduce((sum, e) => sum + (e.hours || 0), 0);

  // Stage dropdown
  let stageOptions = '<option value="">Loading...</option>';
  if (detailStages.length > 0) {
    stageOptions = detailStages.map(s =>
      `<option value="${s.id}"${s.id === task._stageId ? ' selected' : ''}>${esc(s.name)}</option>`
    ).join('');
  } else if (!detailLoading) {
    stageOptions = '<option value="">No stages</option>';
  }

  // State dropdown
  const stateOptions = STATE_ORDER.map(key => {
    const c = STATE_CONFIG[key];
    return `<option value="${key}"${key === stateKey ? ' selected' : ''}>${c.label}</option>`;
  }).join('');

  let html = `
    <div class="detail-header">
      <button class="detail-close-btn" id="detail-close" title="Close (Esc)">
        <svg width="16" height="16" viewBox="0 0 10 10">
          <line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" stroke-width="1.5"/>
          <line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" stroke-width="1.5"/>
        </svg>
      </button>
    </div>

    <div class="detail-body scrollable">
      <div class="detail-name-row">
        <input type="text" class="detail-name-input" id="detail-name" value="${escAttr(task.name)}" data-task-id="${task.id}" />
        <button class="detail-priority-btn${isPriority ? ' active' : ''}" id="detail-priority" data-task-id="${task.id}" title="Toggle priority">
          &#9733;
        </button>
      </div>

      <div class="detail-meta">
        <span class="project-badge">${esc(task.project_name || 'No project')}</span>
        ${deadlinePill}
      </div>

      <div class="detail-fields">
        <div class="detail-field">
          <label>Stage</label>
          <select class="detail-select" id="detail-stage" data-task-id="${task.id}">
            ${stageOptions}
          </select>
        </div>

        <div class="detail-field">
          <label>State</label>
          <div class="detail-state-select-wrap">
            <span class="state-dot-sm" style="background: ${cfg.color}"></span>
            <select class="detail-select" id="detail-state" data-task-id="${task.id}">
              ${stateOptions}
            </select>
          </div>
        </div>

        <div class="detail-field">
          <label>Deadline</label>
          <input type="date" class="detail-date-input" id="detail-deadline" value="${escAttr(task.date_deadline || '')}" data-task-id="${task.id}" />
        </div>
      </div>

      <div class="detail-field">
        <label>Description</label>
        <textarea class="detail-textarea" id="detail-description" data-task-id="${task.id}" placeholder="Add a description...">${esc(task.description || '')}</textarea>
      </div>

      <div class="detail-section">
        <h4>Today's Time</h4>
        ${taskEntries.length > 0
          ? `<div class="detail-time-entries">
              ${taskEntries.map(e => `
                <div class="detail-time-entry">
                  <span class="detail-time-desc">${esc(e.description || e.task_name)}</span>
                  <span class="detail-time-hours">${formatHours(e.hours || 0)}</span>
                </div>
              `).join('')}
              <div class="detail-time-total">
                <span>Total</span>
                <span>${formatHours(taskTotalHours)}</span>
              </div>
            </div>`
          : '<p class="detail-no-entries">No time logged today</p>'
        }
      </div>

      <div class="detail-actions">
        <button class="btn btn-primary btn-sm" id="detail-start-timer" data-task-id="${task.id}"
          data-task-name="${escAttr(task.name)}" data-project-id="${task.project_id || 0}"
          data-project-name="${escAttr(task.project_name || '')}">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21"/></svg>
          Start Timer
        </button>
        <a class="btn btn-secondary btn-sm detail-odoo-link" href="#" id="detail-open-odoo" data-task-id="${task.id}">Open in Odoo</a>
      </div>
    </div>
  `;

  panel.innerHTML = html;

  // Attach event listeners
  attachDetailListeners();
}

function renderDetailTimeEntries() {
  const { detailTask, todayEntries } = store.getState();
  if (!detailTask) return;
  // Minimal re-render of time entries section would be complex,
  // so just re-render the whole detail panel
  renderDetailPanel();
}

// ── Detail Panel Event Listeners ─────────────────────────────────────

function attachDetailListeners() {
  // Close button
  $('#btn-close-detail')?.addEventListener('click', closeDetailPanel);
  $('#detail-close')?.addEventListener('click', closeDetailPanel);

  // Name editing (save on blur or Enter)
  const nameInput = $('#detail-name');
  if (nameInput) {
    nameInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        nameInput.blur();
      }
    });
    nameInput.addEventListener('blur', async () => {
      const newName = nameInput.value.trim();
      const taskId = parseInt(nameInput.dataset.taskId);
      if (!newName || !taskId) return;
      const { detailTask } = store.getState();
      if (detailTask && newName !== detailTask.name) {
        const updatedTask = { ...detailTask, name: newName };
        store.setState({ detailTask: updatedTask });
        updateTaskInList(taskId, { name: newName });
        try {
          await api.updateTaskName(taskId, newName);
          showToast('Name updated', 'success');
        } catch (err) {
          showToast('Failed to update name: ' + String(err));
        }
      }
    });
  }

  // Stage change
  $('#detail-stage')?.addEventListener('change', async (e) => {
    const stageId = parseInt(e.target.value);
    const taskId = parseInt(e.target.dataset.taskId);
    if (!stageId || !taskId) return;
    try {
      await api.updateTaskStage(taskId, stageId);
      const { detailStages } = store.getState();
      const stageName = detailStages.find(s => s.id === stageId)?.name || '';
      showToast(`Stage: "${stageName}"`, 'success');
    } catch (err) {
      showToast('Failed to update stage: ' + String(err));
    }
  });

  // State change
  $('#detail-state')?.addEventListener('change', async (e) => {
    const newState = e.target.value;
    const taskId = parseInt(e.target.dataset.taskId);
    if (!newState || !taskId) return;

    const prevState = store.getState().detailTask?.state;
    // Optimistic update
    const { detailTask } = store.getState();
    if (detailTask) {
      store.setState({ detailTask: { ...detailTask, _state: newState, state: newState } });
      updateTaskInList(taskId, { state: newState });
    }

    try {
      await api.updateTaskState(taskId, newState);
      const label = STATE_CONFIG[newState]?.label || newState;
      showToast(`State: ${label}`, 'success');
      // Re-render kanban to move card
      applyFiltersAndRender();
    } catch (err) {
      // Revert
      if (detailTask) {
        store.setState({ detailTask: { ...detailTask, _state: prevState, state: prevState } });
        updateTaskInList(taskId, { state: prevState });
      }
      showToast('Failed to update state: ' + String(err));
      renderDetailPanel();
    }
  });

  // Deadline change
  $('#detail-deadline')?.addEventListener('change', async (e) => {
    const dateDeadline = e.target.value;
    const taskId = parseInt(e.target.dataset.taskId);
    if (!taskId) return;
    // Optimistic update
    const { detailTask } = store.getState();
    if (detailTask) {
      store.setState({ detailTask: { ...detailTask, date_deadline: dateDeadline || null } });
      updateTaskInList(taskId, { date_deadline: dateDeadline || null });
    }
    applyFiltersAndRender();
    try {
      await api.updateTaskDeadline(taskId, dateDeadline || null);
      showToast('Deadline updated', 'success');
    } catch (err) {
      showToast('Failed to update deadline: ' + String(err));
    }
  });

  // Priority toggle
  $('#detail-priority')?.addEventListener('click', async () => {
    const { detailTask } = store.getState();
    if (!detailTask) return;
    const isPriority = detailTask.priority === '1' || detailTask.priority === 1 || detailTask.is_priority;
    const newPriority = isPriority ? '0' : '1';
    // Optimistic update
    store.setState({ detailTask: { ...detailTask, priority: newPriority, is_priority: newPriority === '1' } });
    updateTaskInList(detailTask.id, { priority: newPriority, is_priority: newPriority === '1' });
    renderDetailPanel();
    applyFiltersAndRender();
    try {
      await api.updateTaskPriority(detailTask.id, newPriority);
      showToast(newPriority === '1' ? 'Marked as priority' : 'Priority removed', 'success');
    } catch (err) {
      showToast('Failed to update priority: ' + String(err));
    }
  });

  // Description (debounced save on blur)
  const descEl = $('#detail-description');
  if (descEl) {
    const debouncedSave = debounce(async () => {
      const desc = descEl.value;
      const taskId = parseInt(descEl.dataset.taskId);
      if (!taskId) return;
      const { detailTask } = store.getState();
      if (detailTask) {
        store.setState({ detailTask: { ...detailTask, description: desc } });
      }
      try {
        await api.updateTaskDescription(taskId, desc);
        showToast('Description saved', 'success');
      } catch (err) {
        showToast('Failed to save description: ' + String(err));
      }
    }, 500);

    descEl.addEventListener('input', debouncedSave);
    descEl.addEventListener('blur', debouncedSave);
  }

  // Start Timer
  $('#detail-start-timer')?.addEventListener('click', async () => {
    const btn = $('#detail-start-timer');
    if (!btn) return;
    const taskId = parseInt(btn.dataset.taskId);
    const taskName = btn.dataset.taskName;
    const projectId = parseInt(btn.dataset.projectId) || 0;
    const projectName = btn.dataset.projectName;
    try {
      await api.startTimer(taskId, taskName, projectId, projectName);
      showToast(`Timer started for "${taskName}"`, 'success');
    } catch (err) {
      showToast('Failed to start timer: ' + String(err));
    }
  });

  // Open in Odoo
  $('#detail-open-odoo')?.addEventListener('click', async (e) => {
    e.preventDefault();
    const taskId = e.target.closest('[data-task-id]')?.dataset.taskId;
    if (!taskId) return;
    const url = await getOdooUrl();
    if (!url) { showToast('Odoo URL not available'); return; }
    const taskUrl = `${url}/web#id=${taskId}&model=project.task&view_type=form`;
    try {
      // Try Tauri opener plugin
      if (window.__TAURI__?.opener?.openUrl) {
        await window.__TAURI__.opener.openUrl(taskUrl);
      } else {
        window.open(taskUrl, '_blank');
      }
    } catch (_) {
      window.open(taskUrl, '_blank');
    }
  });
}

function updateTaskInList(taskId, updates) {
  const { tasks } = store.getState();
  const updatedTasks = tasks.map(t => t.id === taskId ? { ...t, ...updates } : t);
  store.setState({ tasks: updatedTasks });
}

// ── Time Log View ────────────────────────────────────────────────────

function getTimelogDate() {
  return store.getState().timelogDate || todayDate();
}

function formatDateLabel(dateStr) {
  const today = todayDate();
  const d = new Date(dateStr + 'T12:00:00');
  const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const monthNames = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  if (dateStr === today) return 'Today';
  const yesterday = new Date(); yesterday.setDate(yesterday.getDate() - 1);
  if (dateStr === yesterday.toISOString().slice(0, 10)) return 'Yesterday';
  return `${dayNames[d.getDay()]}, ${monthNames[d.getMonth()]} ${d.getDate()}`;
}

// ── Sync indicator ─────────────────────────────────────────────────
function showTimelogSyncIndicator() {
  let el = document.getElementById('timelog-sync-indicator');
  if (!el) {
    el = document.createElement('div');
    el.id = 'timelog-sync-indicator';
    el.className = 'sync-indicator';
    el.innerHTML = '<span class="sync-spinner"></span> Syncing\u2026';
    document.querySelector('.timelog-header')?.appendChild(el);
  }
  el.style.display = '';
}
function hideTimelogSyncIndicator() {
  const el = document.getElementById('timelog-sync-indicator');
  if (el) el.style.display = 'none';
}

async function goToTimelogDate(dateStr) {
  store.setState({ timelogDate: dateStr, timelogMode: 'day', timelogAnalysis: null });
  showTimelogSyncIndicator();
  try {
    const entries = await api.getEntriesForDate(dateStr);
    store.setState({ timelogEntries: entries || [] });
  } catch (err) {
    store.setState({ timelogEntries: [] });
    showToast('Failed to load entries: ' + String(err));
  }
  hideTimelogSyncIndicator();
  // Fetch week context in background (don't block render)
  fetchWeekContext(dateStr);
  renderTimeLog();
}

/** Fetch week totals for the week containing `dateStr` and store them. */
async function fetchWeekContext(dateStr) {
  const ref = new Date(dateStr + 'T12:00:00');
  const day = ref.getDay();
  const monday = new Date(ref);
  monday.setDate(ref.getDate() - ((day + 6) % 7));
  const weekDays = [];
  for (let i = 0; i < 7; i++) {
    const d = new Date(monday);
    d.setDate(monday.getDate() + i);
    weekDays.push(d.toISOString().slice(0, 10));
  }
  // Skip if we already have this week cached
  const existing = store.getState().timelogWeek || [];
  if (existing.length === 7 && existing[0].date === weekDays[0]) return;
  try {
    const results = await Promise.all(weekDays.map(d => api.getEntriesForDate(d).catch(() => [])));
    const weekData = weekDays.map((d, i) => ({
      date: d,
      entries: results[i] || [],
      total: (results[i] || []).reduce((sum, e) => sum + (e.hours || 0), 0),
    }));
    store.setState({ timelogWeek: weekData });
    // Re-render if still in day mode to show updated week chart
    if (store.getState().timelogMode === 'day') renderTimeLog();
  } catch (_) {}
}

async function loadTimelogMonth(year, month) {
  store.setState({ timelogMode: 'month', timelogAnalysis: null });
  try {
    const summary = await api.getMonthlySummary(year, month);
    store.setState({ timelogMonthly: summary });
  } catch (err) {
    store.setState({ timelogMonthly: null });
    showToast('Failed to load monthly summary: ' + String(err));
  }
  renderTimeLog();
}

async function loadTimelogWeek() {
  store.setState({ timelogMode: 'week', timelogAnalysis: null });
  // Get Monday of current week relative to timelogDate
  const ref = new Date(getTimelogDate() + 'T12:00:00');
  const day = ref.getDay();
  const monday = new Date(ref);
  monday.setDate(ref.getDate() - ((day + 6) % 7));

  const weekDays = [];
  for (let i = 0; i < 7; i++) {
    const d = new Date(monday);
    d.setDate(monday.getDate() + i);
    weekDays.push(d.toISOString().slice(0, 10));
  }

  // Fetch entries for each day in parallel
  try {
    const results = await Promise.all(weekDays.map(d => api.getEntriesForDate(d).catch(() => [])));
    const weekData = weekDays.map((d, i) => ({
      date: d,
      entries: results[i] || [],
      total: (results[i] || []).reduce((sum, e) => sum + (e.hours || 0), 0),
    }));
    store.setState({ timelogWeek: weekData });
  } catch (err) {
    showToast('Failed to load week: ' + String(err));
  }
  renderTimeLog();
}

async function analyzeDay() {
  const date = getTimelogDate();
  try {
    const analysis = await api.getDayAnalysis(date);
    store.setState({ timelogAnalysis: analysis });
    renderTimeLog();
  } catch (err) {
    showToast('Failed to analyze day: ' + String(err));
  }
}

function renderTimeLog() {
  const mainEl = $('#main-area');
  if (!mainEl) return;
  const { timelogMode } = store.getState();

  if (timelogMode === 'month') {
    renderTimeLogMonth(mainEl);
  } else if (timelogMode === 'week') {
    renderTimeLogWeek(mainEl);
  } else {
    renderTimeLogDay(mainEl);
  }
}

function renderTimeLogDay(mainEl) {
  const { timelogEntries, timelogAnalysis, timelogWeek } = store.getState();
  const date = getTimelogDate();
  const entries = timelogEntries || [];
  const totalHours = entries.reduce((sum, e) => sum + (e.hours || 0), 0);
  const weekData = timelogWeek || [];

  let html = `
    <div class="timelog-nav">
      <button class="btn-icon" id="tl-prev" title="Previous day">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="15 18 9 12 15 6"/></svg>
      </button>
      <div class="timelog-date-center">
        <span class="timelog-date-label">${formatDateLabel(date)}</span>
        <span class="timelog-date-sub">${date}</span>
      </div>
      <button class="btn-icon" id="tl-next" title="Next day">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
      </button>
    </div>
    <div class="timelog-mode-tabs">
      <button class="timelog-tab active" data-tl-mode="day">Day</button>
      <button class="timelog-tab" data-tl-mode="week">Week</button>
      <button class="timelog-tab" data-tl-mode="month">Month</button>
      <div style="flex:1"></div>
      <button class="btn btn-sm btn-secondary" id="tl-analyze">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M21 12a9 9 0 1 1-9-9"/><path d="M21 3v6h-6"/></svg>
        Analyze
      </button>
    </div>`;

  // Week progress chart (compact version, highlights current day)
  if (weekData.length === 7) {
    const maxH = Math.max(...weekData.map(d => d.total), 1);
    const dayNames = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
    const weekTotal = weekData.reduce((s, d) => s + d.total, 0);
    html += `<div class="timelog-week-chart compact">`;
    for (let i = 0; i < weekData.length; i++) {
      const d = weekData[i];
      const pct = maxH > 0 ? (d.total / maxH * 100) : 0;
      const isActive = d.date === date;
      const isToday = d.date === todayDate();
      html += `<div class="week-bar-col${isToday ? ' today' : ''}${isActive ? ' active' : ''}" data-tl-goto="${d.date}">
        <div class="week-bar-value">${d.total > 0 ? formatHours(d.total) : ''}</div>
        <div class="week-bar-track"><div class="week-bar-fill" style="height:${pct}%"></div></div>
        <div class="week-bar-label">${dayNames[i]}</div>
      </div>`;
    }
    html += `</div>`;
    html += `<div class="timelog-total"><span>Week Total</span><span class="timelog-total-value" style="font-size:12px">${formatHours(weekTotal)}</span></div>`;
  }

  html += `<div class="timelog-total">
      <span>Day Total</span>
      <span class="timelog-total-value">${formatHours(totalHours)}</span>
    </div>`;

  // Day breakdown chart (horizontal bars by task)
  if (entries.length > 0) {
    const maxH = Math.max(...entries.map(e => e.hours || 0), 0.01);
    html += '<div class="timelog-day-chart">';
    for (const e of entries) {
      const pct = Math.min(100, ((e.hours || 0) / totalHours) * 100);
      html += `<div class="day-bar-row">
        <span class="day-bar-label" title="${escAttr(e.project_name || '')}">${esc(e.task_name)}</span>
        <div class="day-bar-track"><div class="day-bar-fill" style="width:${pct}%"></div></div>
        <span class="day-bar-value">${formatHours(e.hours || 0)}</span>
      </div>`;
    }
    html += '</div>';
  }

  // Analysis panel
  if (timelogAnalysis) {
    const a = timelogAnalysis;
    const gapClass = a.gap_hours > 0.25 ? 'text-warning' : a.gap_hours < -0.25 ? 'text-danger' : 'text-success';
    html += `<div class="timelog-analysis">
      <div class="analysis-stats">
        <div class="analysis-stat"><span class="analysis-stat-label">Presence</span><span class="analysis-stat-value">${formatHours(a.total_attendance_hours)}</span></div>
        <div class="analysis-stat"><span class="analysis-stat-label">Logged</span><span class="analysis-stat-value">${formatHours(a.total_timesheet_hours)}</span></div>
        <div class="analysis-stat"><span class="analysis-stat-label">Gap</span><span class="analysis-stat-value ${gapClass}">${(a.gap_hours >= 0 ? '+' : '') + formatHours(Math.abs(a.gap_hours))}</span></div>
      </div>`;
    if (a.suggestions && a.suggestions.length > 0) {
      html += '<div class="analysis-suggestions">';
      for (const s of a.suggestions) {
        html += `<div class="analysis-suggestion">
          <span class="suggestion-message">${esc(s.message)}</span>
          ${s.detail ? `<span class="suggestion-detail">${esc(s.detail)}</span>` : ''}
          ${(s.suggestion_type === 'add_time' && s.task_id) ? `<button class="btn btn-sm btn-primary analysis-apply-btn" data-task-id="${s.task_id}" data-project-id="${s.project_id || 0}" data-task-name="${escAttr(s.task_name)}" data-project-name="${escAttr(s.project_name)}" data-hours="${s.hours}" data-description="${escAttr(s.description || s.task_name)}" data-date="${date}">Apply</button>` : ''}
        </div>`;
      }
      html += '</div>';
    }
    html += '</div>';
  }

  html += '<div class="timelog-list">';
  if (entries.length === 0) {
    html += '<div class="dash-empty"><p>No time entries for this day</p></div>';
  } else {
    // Group by project
    const groups = {};
    for (const e of entries) {
      const proj = e.project_name || 'No project';
      if (!groups[proj]) groups[proj] = { entries: [], total: 0 };
      groups[proj].entries.push(e);
      groups[proj].total += (e.hours || 0);
    }
    for (const [proj, group] of Object.entries(groups).sort((a, b) => a[0].localeCompare(b[0]))) {
      html += `<div style="margin-bottom:4px;font-size:11px;color:var(--text-muted);font-weight:600;text-transform:uppercase;letter-spacing:0.5px;padding:8px 0 2px">${esc(proj)} · ${formatHours(group.total)}</div>`;
      for (const e of group.entries) {
        html += `<div class="timelog-entry">
          <span class="timelog-task-name">${esc(e.task_name)}${e.description ? ' — ' + esc(e.description) : ''}</span>
          <span class="timelog-duration">${formatHours(e.hours || 0)}</span>
        </div>`;
      }
    }
  }
  html += '</div>';
  mainEl.innerHTML = html;
  attachTimelogListeners();
}

function renderTimeLogWeek(mainEl) {
  const weekData = store.getState().timelogWeek || [];
  const totalWeek = weekData.reduce((sum, d) => sum + d.total, 0);
  const maxHours = Math.max(...weekData.map(d => d.total), 1);
  const dayNames = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

  let html = `
    <div class="timelog-mode-tabs">
      <button class="timelog-tab" data-tl-mode="day">Day</button>
      <button class="timelog-tab active" data-tl-mode="week">Week</button>
      <button class="timelog-tab" data-tl-mode="month">Month</button>
    </div>
    <div class="timelog-total">
      <span>Week Total</span>
      <span class="timelog-total-value">${formatHours(totalWeek)}</span>
    </div>
    <div class="timelog-week-chart">`;

  for (let i = 0; i < weekData.length; i++) {
    const d = weekData[i];
    const pct = maxHours > 0 ? (d.total / maxHours * 100) : 0;
    const isToday = d.date === todayDate();
    html += `<div class="week-bar-col${isToday ? ' today' : ''}" data-tl-goto="${d.date}">
      <div class="week-bar-value">${d.total > 0 ? formatHours(d.total) : ''}</div>
      <div class="week-bar-track"><div class="week-bar-fill" style="height:${pct}%"></div></div>
      <div class="week-bar-label">${dayNames[i]}</div>
      <div class="week-bar-date">${d.date.slice(5)}</div>
    </div>`;
  }

  html += '</div><div class="timelog-list">';
  // Show entries for each day that has any
  for (const d of weekData) {
    if (d.entries.length === 0) continue;
    html += `<div style="margin:12px 0 4px;font-size:12px;font-weight:600;color:var(--text-secondary);cursor:pointer" data-tl-goto="${d.date}">${formatDateLabel(d.date)} · ${formatHours(d.total)}</div>`;
    for (const e of d.entries) {
      html += `<div class="timelog-entry">
        <span class="timelog-task-name">${esc(e.task_name)}${e.description ? ' — ' + esc(e.description) : ''}</span>
        <span class="timelog-duration">${formatHours(e.hours || 0)}</span>
      </div>`;
    }
  }
  html += '</div>';
  mainEl.innerHTML = html;
  attachTimelogListeners();
}

function renderTimeLogMonth(mainEl) {
  const monthly = store.getState().timelogMonthly;
  const date = getTimelogDate();
  const year = parseInt(date.slice(0, 4));
  const month = parseInt(date.slice(5, 7));
  const monthNames = ['', 'January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

  let html = `
    <div class="timelog-nav">
      <button class="btn-icon" id="tl-month-prev" title="Previous month">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="15 18 9 12 15 6"/></svg>
      </button>
      <span class="timelog-date-label">${monthNames[month]} ${year}</span>
      <button class="btn-icon" id="tl-month-next" title="Next month">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
      </button>
    </div>
    <div class="timelog-mode-tabs">
      <button class="timelog-tab" data-tl-mode="day">Day</button>
      <button class="timelog-tab" data-tl-mode="week">Week</button>
      <button class="timelog-tab active" data-tl-mode="month">Month</button>
    </div>`;

  if (monthly) {
    html += `<div class="timelog-total"><span>Month Total</span><span class="timelog-total-value">${formatHours(monthly.total_hours)}</span></div>`;
    html += '<div class="timelog-list">';
    const days = monthly.days || [];
    for (const d of days) {
      if (d.hours <= 0) continue;
      html += `<div class="timelog-entry" style="cursor:pointer" data-tl-goto="${d.date}">
        <span class="timelog-task-name">${formatDateLabel(d.date)}</span>
        <span class="timelog-duration">${formatHours(d.hours)}</span>
      </div>`;
    }
    if (days.every(d => d.hours <= 0)) {
      html += '<div class="dash-empty"><p>No hours logged this month</p></div>';
    }
    html += '</div>';
  } else {
    html += '<div class="dash-loading"><div class="dash-spinner"></div><p>Loading...</p></div>';
  }

  mainEl.innerHTML = html;
  attachTimelogListeners();
}

function attachTimelogListeners() {
  // Day navigation
  $('#tl-prev')?.addEventListener('click', () => {
    const d = new Date(getTimelogDate() + 'T12:00:00');
    d.setDate(d.getDate() - 1);
    goToTimelogDate(d.toISOString().slice(0, 10));
  });
  $('#tl-next')?.addEventListener('click', () => {
    const d = new Date(getTimelogDate() + 'T12:00:00');
    d.setDate(d.getDate() + 1);
    goToTimelogDate(d.toISOString().slice(0, 10));
  });
  // Month navigation
  $('#tl-month-prev')?.addEventListener('click', () => {
    const date = getTimelogDate();
    let y = parseInt(date.slice(0, 4)), m = parseInt(date.slice(5, 7));
    m--; if (m < 1) { m = 12; y--; }
    const newDate = `${y}-${String(m).padStart(2, '0')}-01`;
    store.setState({ timelogDate: newDate });
    loadTimelogMonth(y, m);
  });
  $('#tl-month-next')?.addEventListener('click', () => {
    const date = getTimelogDate();
    let y = parseInt(date.slice(0, 4)), m = parseInt(date.slice(5, 7));
    m++; if (m > 12) { m = 1; y++; }
    const newDate = `${y}-${String(m).padStart(2, '0')}-01`;
    store.setState({ timelogDate: newDate });
    loadTimelogMonth(y, m);
  });
  // Mode tabs
  document.querySelectorAll('.timelog-tab[data-tl-mode]').forEach(tab => {
    tab.addEventListener('click', () => {
      const mode = tab.dataset.tlMode;
      if (mode === 'week') { loadTimelogWeek(); }
      else if (mode === 'month') {
        const date = getTimelogDate();
        loadTimelogMonth(parseInt(date.slice(0, 4)), parseInt(date.slice(5, 7)));
      }
      else { goToTimelogDate(getTimelogDate()); }
    });
  });
  // Go to specific date (from week/month views)
  document.querySelectorAll('[data-tl-goto]').forEach(el => {
    el.addEventListener('click', () => goToTimelogDate(el.dataset.tlGoto));
  });
  // Analyze day
  $('#tl-analyze')?.addEventListener('click', analyzeDay);
  // Apply suggestion (log time)
  document.querySelectorAll('.analysis-apply-btn').forEach(btn => {
    btn.addEventListener('click', async () => {
      const taskId = parseInt(btn.dataset.taskId);
      const projectId = parseInt(btn.dataset.projectId) || 0;
      const taskName = btn.dataset.taskName;
      const projectName = btn.dataset.projectName;
      const hours = parseFloat(btn.dataset.hours);
      const description = btn.dataset.description;
      const date = btn.dataset.date;
      btn.disabled = true; btn.textContent = 'Logging...';
      try {
        await api.logTime(taskId, projectId, taskName, projectName, description, hours, date);
        showToast(`Logged ${formatHours(hours)} on "${taskName}"`, 'success');
        goToTimelogDate(date);
      } catch (err) {
        showToast('Failed to log time: ' + String(err));
        btn.disabled = false; btn.textContent = 'Apply';
      }
    });
  });
}

// ── New Task ─────────────────────────────────────────────────────────

let newTaskFormOpen = false;
let newTaskDraft = null;

$('#btn-new-task')?.addEventListener('click', () => {
  toggleNewTaskForm();
});

function prettifyOdooError(err) {
  const s = String(err);
  return s.replace(/^.*?(?:Odoo|AppError|Error)\s*[:(]\s*/i, '').replace(/\)$/, '').trim() || s;
}

// Pick a sensible default project: single active all-tasks project filter, else most-used, else null
function defaultProjectId() {
  const { allTasksFilterProjects } = store.getState();
  if (Array.isArray(allTasksFilterProjects) && allTasksFilterProjects.length === 1) {
    return allTasksFilterProjects[0];
  }
  const usage = getProjectUsage();
  let bestId = null, best = null;
  for (const [id, entry] of Object.entries(usage)) {
    if (!best || (entry.count || 0) > best.count ||
        ((entry.count || 0) === best.count && (entry.lastUsed || 0) > (best.lastUsed || 0))) {
      best = entry; bestId = Number(id);
    }
  }
  return bestId;
}

// Render up to 3 suggested project pills (same usage-sort as populateProjectSelect)
function renderProjectPills(projects) {
  const wrap = $('#new-task-project-pills');
  if (!wrap) return;
  const usage = getProjectUsage();
  const withUsage = [...projects].filter(p => (usage[p.id]?.count || 0) > 0);
  withUsage.sort((a, b) => {
    const ua = usage[a.id], ub = usage[b.id];
    if (ub.count !== ua.count) return ub.count - ua.count;
    return (ub.lastUsed || 0) - (ua.lastUsed || 0);
  });
  const top = withUsage.slice(0, 3);
  wrap.innerHTML = top.map(p =>
    `<button type="button" class="nt-pill" data-project-pill="${p.id}">${esc(p.name)}</button>`
  ).join('');
}

async function createTaskFlow(name, projectId, { description, deadline, priority, startTimer } = {}) {
  const task = await api.createTask(name, projectId);  // backend contract: name + projectId ONLY
  // Fresh Odoo tasks come back with a null state; show the optimistic card in
  // the default column instead of "Other" until the next sync corrects it.
  if (!task.state) task.state = '01_in_progress';
  bumpProjectUsage(projectId);
  // Enrich via existing update commands on the returned id (non-fatal each)
  const extras = [];
  if (description) { extras.push(api.updateTaskDescription(task.id, description).then(() => { task.description = description; })); }
  if (deadline)    { extras.push(api.updateTaskDeadline(task.id, deadline).then(() => { task.date_deadline = deadline; })); }
  if (priority === '1') { extras.push(api.updateTaskPriority(task.id, '1').then(() => { task.priority = '1'; })); }
  await Promise.allSettled(extras);

  const { tasks } = store.getState();
  store.setState({ tasks: [task, ...tasks] });
  // Auto-expand the column this task lands in so it's visible
  const collapsed = new Set(store.getState().collapsedColumns);
  if (collapsed.has(task.state)) { collapsed.delete(task.state); store.setState({ collapsedColumns: collapsed }); }
  applyFiltersAndRender();
  // Flash the new card
  requestAnimationFrame(() => $(`.task-card[data-task-id="${task.id}"]`)?.classList.add('task-card--new'));

  if (startTimer) {
    try { await api.startTimer(task.id, task.name, projectId, task.project_name); } catch (_) {}
  }
  return task;
}

function toggleNewTaskForm() {
  const form = $('#new-task-modal');
  if (!form) return;

  newTaskFormOpen = !newTaskFormOpen;
  form.classList.toggle('open', newTaskFormOpen);

  if (newTaskFormOpen) {
    const { projects } = store.getState();
    populateProjectSelect($('#new-task-project'), projects || []);
    renderProjectPills(projects || []);
    const def = defaultProjectId();
    if (def) $('#new-task-project').value = String(def);
    const nameInput = $('#new-task-name');
    if (nameInput) { nameInput.value = newTaskDraft ? (newTaskDraft.name || '') : ''; nameInput.focus(); }
  } else {
    // Stash a draft on close-without-submit
    const n = $('#new-task-name')?.value.trim();
    newTaskDraft = n ? { name: n } : null;
  }
}

async function submitNewTask({ startTimer } = {}) {
  const name = $('#new-task-name')?.value.trim();
  const projectId = parseInt($('#new-task-project')?.value);
  if (!name) { showToast('Task name is required'); return; }
  if (!projectId) { showToast('Please select a project'); return; }

  const description = $('#new-task-description')?.value.trim() || '';
  const deadline = $('#new-task-deadline')?.value || '';
  const priority = $('#new-task-priority')?.dataset.value === '1' ? '1' : '0';

  const btns = [$('#btn-submit-new-task'), $('#btn-create-start-task')];
  btns.forEach(b => { if (b) { b.disabled = true; b.classList.add('is-loading'); } });
  try {
    await createTaskFlow(name, projectId, { description, deadline, priority, startTimer });
    showToast(`Task "${name}" created`, 'success');
    newTaskDraft = null;
    toggleNewTaskForm();
  } catch (err) {
    showToast('Failed to create task: ' + prettifyOdooError(err));
  } finally {
    btns.forEach(b => { if (b) { b.disabled = false; b.classList.remove('is-loading'); } });
  }
}

$('#btn-submit-new-task')?.addEventListener('click', () => submitNewTask());
$('#btn-create-start-task')?.addEventListener('click', () => submitNewTask({ startTimer: true }));

// Project pill click -> set the select value
$('#new-task-project-pills')?.addEventListener('click', (e) => {
  const pill = e.target.closest('[data-project-pill]');
  if (!pill) return;
  const sel = $('#new-task-project');
  if (sel) sel.value = pill.dataset.projectPill;
  $$('#new-task-project-pills .nt-pill').forEach(p => p.classList.toggle('active', p === pill));
});

// Priority toggle
$('#new-task-priority')?.addEventListener('click', (e) => {
  const btn = e.target.closest('.nt-prio-btn');
  if (!btn) return;
  const group = $('#new-task-priority');
  group.dataset.value = btn.dataset.prio;
  $$('#new-task-priority .nt-prio-btn').forEach(b => b.classList.toggle('active', b === btn));
});

$('#btn-cancel-new-task')?.addEventListener('click', () => {
  if (newTaskFormOpen) toggleNewTaskForm();
});
$('#btn-close-new-task')?.addEventListener('click', () => {
  if (newTaskFormOpen) toggleNewTaskForm();
});
// Close new task modal on backdrop click
$('#new-task-modal')?.addEventListener('click', (e) => {
  if (e.target.id === 'new-task-modal' && newTaskFormOpen) toggleNewTaskForm();
});

$('#new-task-modal')?.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    // Allow newlines in the description textarea
    if (e.target && e.target.id === 'new-task-description') return;
    e.preventDefault();
    submitNewTask();
    return;
  }
  if (e.key === 'Escape') {
    e.preventDefault();
    if (newTaskFormOpen) toggleNewTaskForm();
    return;
  }
  if (e.key === 'Tab') {
    // Focus trap inside the modal card
    const card = $('#new-task-modal .modal-card');
    if (!card) return;
    const focusable = Array.from(card.querySelectorAll(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])'
    )).filter(el => el.offsetParent !== null);
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault(); last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault(); first.focus();
    }
  }
});

// ── Command Palette (Ctrl+K) ─────────────────────────────────────────

let commandPaletteOpen = false;
let commandActiveIndex = -1;
let commandSearchTimeout;
let commandTasks = [];

function openCommandPalette() {
  const palette = $('#cmd-palette');
  if (!palette) return;
  palette.style.display = '';
  commandPaletteOpen = true;
  commandActiveIndex = -1;
  const searchInput = $('#cmd-search');
  if (searchInput) { searchInput.value = ''; searchInput.focus(); }
  loadCommandRecentTasks();
}

function closeCommandPalette() {
  const palette = $('#cmd-palette');
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
    renderCommandResults(commandTasks);
  } catch (_) {
    renderCommandResults([]);
  }
}

function renderCommandResults(tasks, query) {
  const resultsEl = $('#cmd-results');
  if (!resultsEl) return;

  const q = (query || '').trim();
  const createRow = q
    ? `<div class="cmd-item cmd-create-row" data-cmd-create data-name="${escAttr(q)}">
        <span class="cmd-item-dot create">+</span>
        <span class="cmd-item-name">Create "${esc(q)}"…</span>
        <span class="cmd-item-project">opens New Task</span>
      </div>`
    : '';

  if (!tasks || tasks.length === 0) {
    resultsEl.innerHTML = q
      ? createRow
      : '<div class="dash-empty"><p>No tasks found</p></div>';
    commandActiveIndex = -1;
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
    html += `<div class="cmd-group-label">${esc(proj)}</div>`;
    for (const t of groups[proj]) {
      const stateKey = t.state || '';
      const cfg = STATE_CONFIG[stateKey] || { color: 'var(--brand)' };
      html += `<div class="cmd-item${idx === commandActiveIndex ? ' active' : ''}" data-idx="${idx}"
        data-task-id="${t.id}" data-task-name="${escAttr(t.name)}"
        data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
        <span class="cmd-item-dot" style="background: ${cfg.color}"></span>
        <span class="cmd-item-name">${esc(t.name)}</span>
        <span class="cmd-item-project">${esc(t.project_name || '')}</span>
      </div>`;
      idx++;
    }
  }

  resultsEl.innerHTML = html + createRow;
  commandActiveIndex = -1;
}

function updateCommandActive() {
  const items = $$('#cmd-results .cmd-item');
  items.forEach((el, i) => {
    el.classList.toggle('active', i === commandActiveIndex);
  });
  if (commandActiveIndex >= 0 && items[commandActiveIndex]) {
    items[commandActiveIndex].scrollIntoView({ block: 'nearest' });
  }
}

function selectCommandItem(el) {
  if (!el) return;
  if (el.hasAttribute('data-cmd-create')) {
    const name = el.dataset.name || '';
    closeCommandPalette();
    if (!newTaskFormOpen) toggleNewTaskForm();
    const nameInput = $('#new-task-name');
    if (nameInput) nameInput.value = name;
    $('#new-task-project')?.focus();
    return;
  }
  const taskId = parseInt(el.dataset.taskId);
  closeCommandPalette();
  openDetailPanel(taskId);
}

// Command palette search
$('#cmd-search')?.addEventListener('input', (e) => {
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
      // Merge into store.tasks for detail panel access
      const { tasks: existing } = store.getState();
      const existingIds = new Set(existing.map(t => t.id));
      const newTasks = commandTasks.filter(t => !existingIds.has(t.id));
      if (newTasks.length > 0) {
        store.setState({ tasks: [...existing, ...newTasks] });
      }
      renderCommandResults(commandTasks, q);
    } catch (_) {
      renderCommandResults([], q);
    }
  }, 200);
});

// Click on command item
document.addEventListener('click', (e) => {
  const createRow = e.target.closest('[data-cmd-create]');
  if (createRow) {
    const name = createRow.dataset.name || '';
    closeCommandPalette();
    if (!newTaskFormOpen) toggleNewTaskForm();
    const nameInput = $('#new-task-name');
    if (nameInput) nameInput.value = name;
    $('#new-task-project')?.focus();
    return;
  }
  const item = e.target.closest('#cmd-results .cmd-item[data-task-id]');
  if (item) {
    selectCommandItem(item);
    return;
  }
});

$('#cmd-palette')?.addEventListener('click', closeCommandPalette);

// ── Keyboard Shortcuts ───────────────────────────────────────────────

document.addEventListener('keydown', (e) => {
  // Ctrl+K: command palette
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault();
    if (commandPaletteOpen) {
      closeCommandPalette();
    } else {
      openCommandPalette();
    }
    return;
  }

  // Escape: close panels
  if (e.key === 'Escape') {
    if (commandPaletteOpen) {
      e.preventDefault();
      closeCommandPalette();
      return;
    }
    if (store.getState().selectedTask) {
      e.preventDefault();
      closeDetailPanel();
      return;
    }
    if (newTaskFormOpen) {
      e.preventDefault();
      toggleNewTaskForm();
      return;
    }
    return;
  }

  // Command palette navigation
  if (commandPaletteOpen) {
    const items = $$('#cmd-results .cmd-item');
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
    return;
  }

  // R: Refresh (only when not typing in an input)
  if (e.key === 'r' || e.key === 'R') {
    const active = document.activeElement;
    const isTyping = active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.tagName === 'SELECT');
    if (!isTyping && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      refreshTasks();
      return;
    }
  }
});

// ── Bootstrap ─────────────────────────────────────────────────────────

async function init() {
  try {
    const auth = await api.checkAuth();
    if (!auth.authenticated) {
      const mainEl = $('#main-area');
      if (mainEl) mainEl.innerHTML = '<div class="dash-empty"><p>Please log in from the main Pointeuse window first.</p></div>';
      return;
    }
  } catch (e) {
    const mainEl = $('#main-area');
    if (mainEl) mainEl.innerHTML = '<div class="dash-empty"><p>Connection error. Please check the main window.</p></div>';
    return;
  }

  await loadAllData();

  // Auto-refresh every 2 minutes
  setInterval(() => {
    (async () => {
      try {
        const tasks = await api.getMyTasks();
        const entries = await api.getTodayEntries();
        store.setState({ tasks: tasks || [], todayEntries: entries || [] });
        applyFiltersAndRender();
      } catch (_) {}
    })();
  }, 2 * 60 * 1000);
}

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
      store.setState({ tasks });
      applyFiltersAndRender();
    }
  });
  // Listen for all_tasks_refreshed from background refresh
  window.__TAURI__?.event?.listen('all_tasks_refreshed', (event) => {
    const tasks = event.payload;
    if (Array.isArray(tasks)) {
      store.setState({ allTasksData: tasks, allTasksLoading: false });
      const { activeSection } = store.getState();
      if (activeSection === 'all-tasks') applyFiltersAndRender();
    }
  });
} catch (_) {}

// Listen for cache refresh events from backend background sync
try {
  window.__TAURI__?.event?.listen('entries_refreshed', (event) => {
    const { date, entries } = event.payload;
    const currentDate = getTimelogDate();
    const { timelogMode } = store.getState();
    if (timelogMode === 'day' && date === currentDate) {
      store.setState({ timelogEntries: entries || [] });
      renderTimeLog();
    }
    hideTimelogSyncIndicator();
  });
  window.__TAURI__?.event?.listen('monthly_refreshed', (event) => {
    const summary = event.payload;
    const { timelogMode } = store.getState();
    if (timelogMode === 'month') {
      store.setState({ timelogMonthly: summary });
      renderTimeLog();
    }
    hideTimelogSyncIndicator();
  });
  window.__TAURI__?.event?.listen('analysis_refreshed', (event) => {
    const analysis = event.payload;
    const currentDate = getTimelogDate();
    if (analysis.date === currentDate) {
      store.setState({ timelogAnalysis: analysis });
      renderTimeLog();
    }
    hideTimelogSyncIndicator();
  });
} catch (_) {}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
