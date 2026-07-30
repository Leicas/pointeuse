// Haply Time — Manual Timesheet Entry Composer
//
// Shared ES module used by BOTH windows (main.js / tray and dashboard.js).
// This is the one deliberate convention break in this codebase: createStore,
// esc/escAttr and the api object are duplicated across the two entry points,
// but this component WRITES to the system of record, and two copies of ~700
// lines of write-path UI would drift. Both entry points already use
// <script type="module">, so there is no bundler and no CDN involved.
//
// The module owns its own DOM (it appends `.ec-backdrop` to document.body on
// first open, so neither HTML file needs new markup) and its own `.ec-*` CSS
// namespace, which lives in styles/main.css because BOTH windows load it.
//
// One visibility class — `.ec-backdrop.open` — is used in both windows, rather
// than `.open` on the dashboard and `.visible` in the tray. Deliberate, noted.

// ── Constants ─────────────────────────────────────────────────────────

/** Same tolerance the backend sync path uses in find_duplicate (~72 seconds). */
export const HOURS_TOLERANCE = 0.02;

const MIN_HOURS = 0.01;
const MAX_HOURS = 24;
const DRAFT_KEY = 'haplyTime.entryDraft';
const SUBMIT_GUARD_MS = 3000;
const UNDO_MS = 10000;

// ── Duration parser ───────────────────────────────────────────────────
//
// Free text, not a number spinner: a spinner cannot express "1h30" and forces
// mental arithmetic on the most error-prone field in the form.
//
// A bare number is ALWAYS hours. We never guess that "90" meant minutes — a
// silent 10x error is exactly the bad data this whole feature exists to stop.
//
// This project has no test harness of any kind, so the table below is the test
// suite. Every row is `input → result`; keep it in sync with the code.
//
//   ''          → empty            | '  '        → empty
//   '1:30'      → 1.5              | ':45'       → 0.75
//   '0:30'      → 0.5              | '2:05'      → 2.0833…
//   '1:60'      → error (mm > 59)  | '1:5'       → 1.0833…
//   '2h'        → 2                | '1h30'      → 1.5
//   '1h30m'     → 1.5              | '1.5h'      → 1.5
//   '0.25h'     → 0.25             | '2 h'       → 2      (inner space stripped)
//   '90m'       → 1.5              | '45min'     → 0.75
//   '30m'       → 0.5              | '0.5m'      → error  (< 0.01 h floor)
//   '1.5'       → 1.5              | '8'         → 8
//   '24'        → 24               | '25'        → error  (> 24 h)
//   '90'        → error  (> 24 h — NOT silently read as 90 minutes)
//   '0'         → error  (< 0.01 h floor)
//   '1,5'       → 1.5   (comma decimal)
//   '  2H  '    → 2     (trimmed, lowercased)
//   '.5'        → error (a digit is required before the decimal point)
//   '-1'        → error | 'abc' → error | '1h2h' → error | '1:30:00' → error
//
/**
 * Parse a human duration into fractional hours.
 * Pure: no DOM, no IO. Returns {ok:true,hours} | {ok:false,empty?,error}.
 */
export function parseDuration(raw) {
  const s = String(raw == null ? '' : raw).trim().toLowerCase().replace(/,/g, '.').replace(/\s+/g, '');
  if (!s) return { ok: false, empty: true, error: '' };

  const bad = { ok: false, error: "Couldn't read that. Try 1.5, 1h30, or 90m." };
  let hours = null;
  let m;

  if ((m = /^(\d*):([0-5]?\d)$/.exec(s))) {
    hours = (m[1] ? parseInt(m[1], 10) : 0) + parseInt(m[2], 10) / 60;
  } else if ((m = /^(\d+(?:\.\d+)?)h(?:(\d+)m?)?$/.exec(s))) {
    hours = parseFloat(m[1]) + (m[2] ? parseInt(m[2], 10) / 60 : 0);
  } else if ((m = /^(\d+(?:\.\d+)?)m(?:in)?$/.exec(s))) {
    hours = parseFloat(m[1]) / 60;
  } else if ((m = /^(\d+(?:\.\d+)?)$/.exec(s))) {
    hours = parseFloat(m[1]);
    // A bare number is hours. If it cannot be hours, reject rather than guess.
    if (hours > MAX_HOURS) {
      return { ok: false, error: `${m[1]} is over 24 h. Write 1h30 or 90m if you meant minutes.` };
    }
  } else {
    return bad;
  }

  if (!Number.isFinite(hours)) return bad;
  if (hours < MIN_HOURS) return { ok: false, error: 'Must be at least 0.01 h (about 36 seconds).' };
  if (hours > MAX_HOURS) return { ok: false, error: "That's more than 24 h." };
  return { ok: true, hours };
}

// ── Reconciliation rule ───────────────────────────────────────────────

/**
 * THE RECONCILIATION RULE — written once here, used by both windows.
 *
 * An optimistically-spliced row is DROPPED as soon as an authoritative row with
 * the same (task_id, date, hours within HOURS_TOLERANCE) arrives, otherwise the
 * day double-counts for one refresh cycle.
 *
 * A QUEUED row is never matched this way. It carries its own `pending_id` and
 * disappears from get_pending_for_date the moment its status becomes 'synced',
 * so the backend already de-duplicates it. Matching it heuristically would hide
 * a legitimate second identical block of time — the exact "Second block — log
 * anyway" case this feature advertises — from both the day list and the
 * "+X queued" figure, with no way to see or discard it.
 *
 * Never write optimistic rows into cached_timesheet_entries: its PRIMARY KEY is
 * odoo_id alone, id-less rows are stored as 0 and collide across dates, and
 * spawn_entries_refresh full-replaces the whole date fire-and-forget. Optimistic
 * rows live in frontend state only, until entries_refreshed lands.
 *
 * @param {Array} authoritative rows from get_entries_for_date / entries_refreshed
 * @param {Array} optimistic    locally-held rows (optimistic or pending)
 * @returns {Array} the optimistic rows that are still NOT represented upstream
 */
export function reconcileEntries(authoritative, optimistic) {
  const real = authoritative || [];
  return (optimistic || []).filter(o => o.pending_id != null || !real.some(a =>
    (a.task_id || 0) === (o.task_id || 0) &&
    a.date === o.date &&
    Math.abs((a.hours || 0) - (o.hours != null ? o.hours : o.duration_hours || 0)) < HOURS_TOLERANCE
  ));
}

// ── Local escaping (mirrors esc/escAttr in both entry points) ─────────

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

// ── Small shared helpers ──────────────────────────────────────────────

function todayDate() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function addDays(dateStr, n) {
  const d = new Date(dateStr + 'T12:00:00');
  d.setDate(d.getDate() + n);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function formatHours(hours) {
  const abs = Math.abs(hours || 0);
  const sign = (hours || 0) < 0 ? '-' : '';
  const h = Math.floor(abs);
  const m = Math.round((abs - h) * 60);
  return `${sign}${h}h ${m}m`;
}

function formatClock(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}:${String(m).padStart(2, '0')}`;
}

function dayLabel(dateStr) {
  if (dateStr === todayDate()) return 'Today';
  if (dateStr === addDays(todayDate(), -1)) return 'Yesterday';
  const d = new Date(dateStr + 'T12:00:00');
  return ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][d.getDay()];
}

/** Strip the `Odoo error: ` / `AppError: ` prefixes the backend Display adds. */
export function prettifyOdooError(err) {
  let s = String(err == null ? '' : err);
  s = s.replace(/^(Odoo error|AppError|Error)\s*:\s*/i, '');
  return s.trim();
}

/** Day-analysis cache so opening the composer doesn't hammer Odoo. */
const analysisCache = new Map(); // date -> { at, analysis }
const ANALYSIS_TTL = 60000;

// ── Row-action gating (used by both windows' day lists) ───────────────

/**
 * MANDATORY GATING. TimesheetEntry.id is an Odoo account.analytic.line id when
 * `source` is 'odoo'/'cache', but a timesheet_log ROWID when `source` is
 * 'local'. Treating them alike would unlink an unrelated record.
 */
export function entryCapabilities(entry) {
  if (entry && entry._pending) {
    return { kind: 'pending', canEdit: true, canDelete: true, canDuplicate: false };
  }
  const src = entry && entry.source;
  const hasOdooId = entry && entry.id != null && (src === 'odoo' || src === 'cache');
  if (hasOdooId) return { kind: 'odoo', canEdit: true, canDelete: true, canDuplicate: true };
  return { kind: 'local', canEdit: false, canDelete: false, canDuplicate: true };
}

const ICON_EDIT = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>';
const ICON_COPY = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const ICON_DEL = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/></svg>';

/** Hover/focus-within action group for one day-list row. */
export function rowActionsHtml(entry) {
  const cap = entryCapabilities(entry);
  if (cap.kind === 'local') {
    return `<span class="ec-row-actions">
      <button type="button" class="btn-icon ec-row-btn" data-ec-act="duplicate" aria-label="Log time on this task again" title="Log time on this task again">${ICON_COPY}</button>
      <button type="button" class="btn-icon ec-row-btn" data-ec-act="edit" aria-label="Edit entry" aria-disabled="true" disabled title="Only known locally — reconnect to Odoo to edit">${ICON_EDIT}</button>
    </span>`;
  }
  if (cap.kind === 'pending') {
    return `<span class="ec-row-actions">
      <button type="button" class="btn-icon ec-row-btn" data-ec-act="edit" aria-label="Fix queued entry" title="Fix queued entry">${ICON_EDIT}</button>
      <button type="button" class="btn-icon ec-row-btn ec-row-btn-danger" data-ec-act="delete" aria-label="Discard queued entry" title="Discard queued entry">${ICON_DEL}</button>
    </span>`;
  }
  return `<span class="ec-row-actions">
    <button type="button" class="btn-icon ec-row-btn" data-ec-act="edit" aria-label="Edit entry" title="Edit entry">${ICON_EDIT}</button>
    <button type="button" class="btn-icon ec-row-btn" data-ec-act="duplicate" aria-label="Duplicate entry" title="Duplicate entry">${ICON_COPY}</button>
    <button type="button" class="btn-icon ec-row-btn ec-row-btn-danger" data-ec-act="delete" aria-label="Delete entry" title="Delete entry">${ICON_DEL}</button>
  </span>`;
}

/** data-* attributes + tabindex for one day-list row. */
export function rowAttrs(entry) {
  const cap = entryCapabilities(entry);
  const parts = [
    `data-source="${escAttr(entry._pending ? 'pending' : (entry.source || ''))}"`,
    `tabindex="0"`,
  ];
  if (entry._pending) {
    parts.push(`data-pending-id="${escAttr(entry.pending_id != null ? entry.pending_id : '')}"`);
  } else if (entry.id != null) {
    parts.push(`data-entry-id="${escAttr(entry.id)}"`);
  }
  if (cap.kind === 'local') parts.push('title="Only known locally — reconnect to Odoo to edit"');
  return parts.join(' ');
}

/** Badge for a queued/rejected/duplicate pending row. */
export function pendingBadgeHtml(p) {
  const map = {
    pending: ['Queued', 'warn'],
    syncing: ['Syncing', 'warn'],
    failed: [`Retrying (${p.retry_count || 0})`, 'warn'],
    duplicate: ['Duplicate', 'warn'],
    rejected: ['Rejected', 'err'],
  };
  const [label, tone] = map[p.status] || ['Queued', 'warn'];
  return `<span class="pending-badge is-${tone}">${esc(label)}</span>`;
}

/** Normalise a PendingTimesheet into the shape the day lists render. */
export function pendingToRow(p) {
  return {
    _pending: true,
    pending_id: p.id,
    id: null,
    task_id: p.task_id,
    task_name: p.task_name || `Task #${p.task_id}`,
    project_id: p.project_id,
    project_name: p.project_name || '',
    description: p.description || '',
    hours: p.duration_hours,
    date: p.date,
    source: 'pending',
    status: p.status,
    retry_count: p.retry_count,
    last_error: p.last_error,
  };
}

// ══════════════════════════════════════════════════════════════════════
//  COMPOSER
// ══════════════════════════════════════════════════════════════════════

/**
 * @param {object}   o
 * @param {Function} o.invoke        the window's wrapped Tauri invoke
 * @param {Function} o.showToast     (msg, type) => void
 * @param {string}   o.variant       'dashboard' | 'tray' (sizing/density only)
 * @param {Function} o.getViewedDate () => 'YYYY-MM-DD' currently on screen
 * @param {Function} o.onChanged     (date, info) => void, after any mutation
 */
export function createEntryComposer({ invoke, showToast, variant = 'dashboard', getViewedDate, onChanged }) {
  let root = null;
  let open = false;
  let returnFocusTo = null;

  // Form state
  let st = blankState();

  // async bookkeeping
  let searchTimer = null;
  let preflightTimer = null;
  let preflightToken = 0;
  let activeIndex = -1;
  let lastSubmit = { key: '', at: 0 };
  let discardArmed = false;

  function blankState() {
    return {
      mode: 'create',          // 'create' | 'edit' | 'repair'
      date: todayDate(),
      originalDate: null,
      task: null,              // { id, name, project_id, project_name }
      projectIdOverride: null,
      resolvingProject: false, // looking up a recent chip's missing project_id
      durationText: '',
      description: '',
      allowDuplicate: false,
      odooId: null,
      pendingId: null,
      preflight: null,
      preflightBusy: false,
      rejected: null,          // { message, isPermanent }
      submitting: false,
      descBlank: true,         // last-seen blankness of the description field
      dayTotals: {},           // date -> hours (for the date chips)
      gapHours: 0,
      initial: '',             // serialized snapshot, for the edit "changed?" test
    };
  }

  function q(sel) { return root ? root.querySelector(sel) : null; }

  // ── Mount ───────────────────────────────────────────────────────────

  function mount() {
    if (root) return;
    root = document.createElement('div');
    root.className = 'ec-backdrop';
    root.id = 'ec-backdrop';
    root.innerHTML = `
      <div class="ec-card${variant === 'tray' ? ' tray' : ''}" role="dialog" aria-modal="true" aria-labelledby="ec-title">
        <div class="ec-header">
          <h2 class="ec-title" id="ec-title">Add time entry</h2>
          <span class="ec-destination" id="ec-destination"></span>
          <button type="button" class="btn-icon ec-close" id="ec-close" aria-label="Close">
            <svg width="14" height="14" viewBox="0 0 10 10" aria-hidden="true"><line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" stroke-width="1.5"/><line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" stroke-width="1.5"/></svg>
          </button>
        </div>

        <div class="ec-body" id="ec-body">
          <div class="ec-field">
            <label class="ec-label" for="ec-task-input" id="ec-task-label">Task</label>
            <div id="ec-task-area"></div>
            <div class="ec-project-line" id="ec-project-line"></div>
          </div>

          <div class="ec-field">
            <span class="ec-label" id="ec-date-label">Date</span>
            <div class="ec-date-row" id="ec-date-row" role="group" aria-labelledby="ec-date-label"></div>
            <input type="date" class="ec-date" id="ec-date" aria-label="Entry date">
          </div>

          <div class="ec-field">
            <label class="ec-label" for="ec-duration">Duration</label>
            <input type="text" class="ec-input ec-duration" id="ec-duration"
                   inputmode="text" autocomplete="off" spellcheck="false"
                   placeholder="1h30, 1.5 or 90m"
                   aria-describedby="ec-duration-echo">
            <div class="ec-duration-quick" id="ec-duration-quick" role="group" aria-label="Add to duration"></div>
            <div class="ec-duration-echo" id="ec-duration-echo" aria-live="polite"></div>
          </div>

          <div class="ec-field">
            <label class="ec-label" for="ec-description">Description <span class="ec-optional">optional</span></label>
            <textarea class="ec-input ec-textarea" id="ec-description" rows="2"></textarea>
          </div>
        </div>

        <div class="ec-checks" id="ec-checks" aria-live="polite"></div>
        <div class="ec-footer" id="ec-footer"></div>
      </div>`;
    document.body.appendChild(root);
    wireStaticListeners();
  }

  // ── Static listeners (survive every partial re-render) ──────────────

  function wireStaticListeners() {
    q('#ec-close').addEventListener('click', () => requestClose());

    root.addEventListener('mousedown', (e) => {
      if (e.target === root) requestClose();
    });

    // Date
    q('#ec-date').addEventListener('change', (e) => {
      if (!e.target.value) return;
      st.date = e.target.value;
      renderDateRow();
      loadGap();
      schedulePreflight();
    });

    q('#ec-date-row').addEventListener('click', (e) => {
      const btn = e.target.closest('[data-ec-date]');
      if (!btn) return;
      st.date = btn.dataset.ecDate;
      q('#ec-date').value = st.date;
      renderDateRow();
      // renderDateRow() replaced the chip that was just clicked, so focus fell
      // to <body> — from there Tab escapes the dialog entirely. Put it back on
      // the equivalent chip in the new markup.
      refocusDateChip();
      loadGap();
      schedulePreflight();
    });

    // Duration
    const dur = q('#ec-duration');
    dur.addEventListener('input', () => {
      st.durationText = dur.value;
      renderDurationEcho();
      renderFooter();
      schedulePreflight();
    });

    q('#ec-duration-quick').addEventListener('click', (e) => {
      const btn = e.target.closest('[data-ec-add]');
      if (!btn) return;
      const add = parseFloat(btn.dataset.ecAdd);
      if (btn.dataset.ecSet === '1') {
        st.durationText = trimNum(add);
      } else {
        // Quick chips ADD rather than replace: clicking 30m twice gives 1h.
        const cur = parseDuration(st.durationText);
        st.durationText = trimNum((cur.ok ? cur.hours : 0) + add);
      }
      dur.value = st.durationText;
      renderDurationEcho();
      renderFooter();
      schedulePreflight();
    });

    // Description
    q('#ec-description').addEventListener('input', (e) => {
      st.description = e.target.value;
      // .ec-checks is an aria-live region and renderChecks() replaces the whole
      // strip, so re-rendering per keystroke makes assistive tech re-announce
      // every row while the user types. Only the "description will be …" row
      // depends on this field, so re-render only when blankness actually flips.
      const blank = !st.description.trim();
      if (blank !== st.descBlank) {
        st.descBlank = blank;
        renderChecks();
      }
      renderFooter();
    });

    // Task area (delegated — the input is replaced when a task is picked)
    const area = q('#ec-task-area');
    area.addEventListener('input', (e) => {
      if (e.target.id !== 'ec-task-input') return;
      scheduleSearch(e.target.value.trim());
    });
    area.addEventListener('click', (e) => {
      if (e.target.closest('.ec-task-clear')) { clearTask(); return; }
      const opt = e.target.closest('.ec-result[data-task-id]');
      if (opt) selectTaskFromEl(opt);
      const chip = e.target.closest('.recent-chip[data-task-id]');
      if (chip) selectTaskFromEl(chip);
    });
    area.addEventListener('change', (e) => {
      if (e.target.id !== 'ec-project') return;
      st.projectIdOverride = parseInt(e.target.value, 10) || null;
      renderFooter();
      schedulePreflight();
    });

    // Checks strip + footer + rejection panel actions (all delegated)
    q('#ec-checks').addEventListener('click', onChecksClick);
    q('#ec-footer').addEventListener('click', onFooterClick);

    // Keyboard: this listener is scoped to the card, so it runs before the
    // window-level cascade only for events originating inside the composer.
    root.addEventListener('keydown', onCardKeydown);

    // Safety net for the one case the card listener cannot see: a partial
    // re-render destroyed the element that had focus, so focus fell to <body>
    // and keydown no longer reaches the card at all. Without this, Tab walks
    // straight out of the open dialog into the window behind the backdrop.
    document.addEventListener('keydown', onEscapedKeydown);
  }

  /** Put focus back on the chip for the currently-selected date. */
  function refocusDateChip() {
    root.querySelector(`#ec-date-row [data-ec-date="${st.date}"]`)?.focus();
  }

  function trimNum(n) {
    const v = Math.max(0, Math.min(MAX_HOURS, n));
    return String(Math.round(v * 1000) / 1000);
  }

  // ── Open / close ────────────────────────────────────────────────────

  function openComposer(seed = {}) {
    mount();
    returnFocusTo = document.activeElement;
    st = blankState();
    discardArmed = false;

    st.mode = seed.mode || 'create';
    st.date = seed.date || (getViewedDate ? getViewedDate() : todayDate()) || todayDate();
    st.originalDate = st.date;
    st.odooId = seed.odooId != null ? seed.odooId : null;
    st.pendingId = seed.pendingId != null ? seed.pendingId : null;
    st.allowDuplicate = !!seed.allowDuplicate;
    st.description = seed.description || '';
    if (seed.taskId) {
      st.task = {
        id: seed.taskId,
        name: seed.taskName || `Task #${seed.taskId}`,
        project_id: seed.projectId || 0,
        project_name: seed.projectName || '',
      };
    }
    if (seed.durationHours != null && seed.durationHours > 0) {
      st.durationText = trimNum(seed.durationHours);
    }

    // Offer back a stashed draft, but only for a fresh blank create.
    if (st.mode === 'create' && !seed.taskId && !seed.durationHours) {
      const draft = readDraft();
      if (draft && draft.date === st.date) {
        st.task = draft.task || null;
        st.durationText = draft.durationText || '';
        st.description = draft.description || '';
      }
    }

    st.initial = snapshot();
    st.descBlank = !st.description.trim();

    // A discard bar left over from a previous session would otherwise reappear.
    root.querySelector('#ec-discard')?.remove();

    root.classList.add('open');
    open = true;

    q('#ec-title').textContent =
      st.mode === 'edit' ? 'Edit entry'
      : st.mode === 'repair' ? 'Fix rejected entry'
      : 'Add time entry';

    q('#ec-date').value = st.date;
    q('#ec-duration').value = st.durationText;
    q('#ec-description').value = st.description;

    renderTaskArea();
    renderDateRow();
    renderQuickChips();
    renderDurationEcho();
    renderChecks();
    renderFooter();

    loadDayTotals();
    loadGap();
    schedulePreflight();

    // Focus rule: the combobox when no task is known, the DURATION field when
    // the task came prefilled — that is the majority of entry points, and it
    // removes a keystroke from the most common flow.
    setTimeout(() => {
      if (st.task) q('#ec-duration')?.focus();
      else q('#ec-task-input')?.focus();
    }, 0);
  }

  function closeComposer() {
    if (!root) return;
    root.classList.remove('open');
    open = false;
    discardArmed = false;
    clearTimeout(searchTimer);
    clearTimeout(preflightTimer);
    preflightToken++;
    const el = returnFocusTo;
    returnFocusTo = null;
    // Focus restore — no other modal in this codebase does this, and the entry
    // points live inside scrolling lists, so it genuinely matters here.
    if (el && typeof el.focus === 'function' && document.contains(el)) {
      try { el.focus(); } catch (_) {}
    }
  }

  function snapshot() {
    return JSON.stringify({
      t: st.task ? st.task.id : 0,
      p: currentProjectId(),
      d: st.date,
      h: st.durationText,
      x: st.description,
    });
  }

  function isDirty() {
    return snapshot() !== st.initial;
  }

  function requestClose() {
    if (!isDirty()) { closeComposer(); return; }
    if (!discardArmed) { armDiscard(); return; }
    stashDraft();
    closeComposer();
  }

  function armDiscard() {
    discardArmed = true;
    const bar = document.createElement('div');
    bar.className = 'ec-discard';
    bar.id = 'ec-discard';
    bar.innerHTML = `<span>Discard this entry?</span>
      <button type="button" class="btn btn-sm btn-danger-outline" data-ec-discard="yes">Discard</button>
      <button type="button" class="btn btn-sm btn-secondary" data-ec-discard="no">Keep editing</button>`;
    q('#ec-footer').before(bar);
    bar.addEventListener('click', (e) => {
      const b = e.target.closest('[data-ec-discard]');
      if (!b) return;
      if (b.dataset.ecDiscard === 'yes') { stashDraft(); closeComposer(); }
      else { discardArmed = false; bar.remove(); q('#ec-duration')?.focus(); }
    });
    bar.querySelector('[data-ec-discard="no"]').focus();
  }

  function stashDraft() {
    try {
      if (st.mode !== 'create' || !isDirty()) { localStorage.removeItem(DRAFT_KEY); return; }
      localStorage.setItem(DRAFT_KEY, JSON.stringify({
        date: st.date, task: st.task, durationText: st.durationText, description: st.description,
      }));
    } catch (_) {}
  }

  function readDraft() {
    try { return JSON.parse(localStorage.getItem(DRAFT_KEY) || 'null'); } catch (_) { return null; }
  }

  function clearDraft() {
    try { localStorage.removeItem(DRAFT_KEY); } catch (_) {}
  }

  // ── Task combobox ───────────────────────────────────────────────────

  function currentProjectId() {
    if (st.projectIdOverride) return st.projectIdOverride;
    return (st.task && st.task.project_id) || 0;
  }

  function renderTaskArea() {
    const area = q('#ec-task-area');
    if (st.task) {
      area.innerHTML = `<div class="ec-task-chip">
        <span class="ec-task-chip-name">${esc(st.task.name)}</span>
        <button type="button" class="btn-icon ec-task-clear" aria-label="Change task" title="Change task">
          <svg width="12" height="12" viewBox="0 0 10 10" aria-hidden="true"><line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" stroke-width="1.5"/><line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" stroke-width="1.5"/></svg>
        </button>
      </div>`;
    } else {
      area.innerHTML = `
        <input type="text" class="ec-input" id="ec-task-input" role="combobox"
               aria-expanded="false" aria-controls="ec-results" aria-autocomplete="list"
               autocomplete="off" spellcheck="false" placeholder="Search tasks…">
        <div class="ec-recents" id="ec-recents"></div>
        <div class="ec-results" id="ec-results" role="listbox" aria-label="Task results"></div>`;
      loadRecents();
    }
    renderProjectLine();
  }

  function renderProjectLine() {
    const el = q('#ec-project-line');
    if (!st.task) { el.innerHTML = ''; return; }
    // Project is DERIVED from the task. That is a correctness decision: today
    // the frontend passes `entry.project_id || 0` and nothing validates it, so
    // project_id 0 reaches Odoo. Deriving it makes that unrepresentable.
    if (st.task.project_id) {
      el.innerHTML = `<span class="project-badge">${esc(st.task.project_name || 'Project')}</span>`;
      return;
    }
    if (st.resolvingProject) {
      // Avoid flashing the manual <select> (and a get_projects round-trip)
      // while we are still resolving the task's own project.
      el.innerHTML = '<span class="ec-label" style="margin:0">Finding project…</span>';
      return;
    }
    el.innerHTML = `<label class="ec-label" for="ec-project">Project (required)</label>
      <select class="select-input" id="ec-project"><option value="">Loading…</option></select>`;
    invoke('get_projects').then(projects => {
      const sel = q('#ec-project');
      if (!sel) return;
      const sorted = [...(projects || [])].sort((a, b) => a.name.localeCompare(b.name));
      sel.innerHTML = '<option value="">Select a project…</option>' +
        sorted.map(p => `<option value="${p.id}"${p.id === st.projectIdOverride ? ' selected' : ''}>${esc(p.name)}</option>`).join('');
    }).catch(() => {
      const sel = q('#ec-project');
      if (sel) sel.innerHTML = '<option value="">Projects unavailable offline</option>';
    });
  }

  async function loadRecents() {
    try {
      const tasks = await invoke('get_recent_tasks');
      const el = q('#ec-recents');
      if (!el || st.task) return;
      el.innerHTML = (tasks || []).slice(0, 6).map(t => `
        <button type="button" class="recent-chip" data-task-id="${t.task_id != null ? t.task_id : t.id}"
          data-task-name="${escAttr(t.task_name || t.name)}"
          data-project-id="${t.project_id || 0}"
          data-project-name="${escAttr(t.project_name || '')}">${esc(t.task_name || t.name)}</button>`).join('');
    } catch (_) {}
  }

  function scheduleSearch(query) {
    clearTimeout(searchTimer);
    const recents = q('#ec-recents');
    if (recents) recents.style.display = query ? 'none' : '';
    if (!query) { setResults([]); return; }
    // "Still typing", "nothing matched" and "the search failed" must not all
    // render as the same blank space — the user cannot tell them apart.
    setResultsMessage('Searching…');
    searchTimer = setTimeout(async () => {
      try {
        // Server-side search — the cache holds ~2400 tasks across 110 projects,
        // so a client-side list is not an option.
        const tasks = await invoke('search_tasks', { query, projectId: null });
        if (tasks && tasks.length) setResults(tasks);
        else setResultsMessage(`No tasks match “${query}”`);
      } catch (_) {
        setResultsMessage("Couldn't search tasks — check the connection", 'is-err');
      }
    }, 200);
  }

  /** A non-selectable status row inside the results list. */
  function setResultsMessage(text, cls = '') {
    const el = q('#ec-results');
    const input = q('#ec-task-input');
    if (!el) return;
    activeIndex = -1;
    el.innerHTML = `<div class="ec-result-msg ${cls}" role="presentation">${esc(text)}</div>`;
    if (input) { input.setAttribute('aria-expanded', 'false'); input.removeAttribute('aria-activedescendant'); }
  }

  function setResults(tasks) {
    const el = q('#ec-results');
    const input = q('#ec-task-input');
    if (!el) return;
    activeIndex = -1;
    if (!tasks.length) {
      el.innerHTML = '';
      if (input) { input.setAttribute('aria-expanded', 'false'); input.removeAttribute('aria-activedescendant'); }
      return;
    }
    const groups = {};
    for (const t of tasks) {
      const p = t.project_name || 'No project';
      (groups[p] = groups[p] || []).push(t);
    }
    let html = '';
    let i = 0;
    for (const p of Object.keys(groups).sort((a, b) => (a === 'No project') - (b === 'No project') || a.localeCompare(b))) {
      // The project heading is wrapped in role=group rather than dropped loose
      // inside the listbox, which would be an invalid listbox structure.
      html += `<div role="group" aria-label="${escAttr(p)}">
        <div class="ec-group-label" aria-hidden="true">${esc(p)}</div>`;
      for (const t of groups[p]) {
        html += `<div class="ec-result" role="option" aria-selected="false" id="ec-opt-${i}" data-idx="${i}"
          data-task-id="${t.id}" data-task-name="${escAttr(t.name)}"
          data-project-id="${t.project_id || 0}" data-project-name="${escAttr(t.project_name || '')}">
          <span class="ec-result-name">${esc(t.name)}</span>
        </div>`;
        i++;
      }
      html += '</div>';
    }
    el.innerHTML = html;
    if (input) input.setAttribute('aria-expanded', 'true');
  }

  function moveActive(delta) {
    const items = root.querySelectorAll('#ec-results .ec-result');
    if (!items.length) return;
    activeIndex = (activeIndex + delta + items.length) % items.length;
    items.forEach((el, i) => {
      const on = i === activeIndex;
      el.classList.toggle('active', on);
      el.setAttribute('aria-selected', on ? 'true' : 'false');
    });
    items[activeIndex].scrollIntoView({ block: 'nearest' });
    q('#ec-task-input')?.setAttribute('aria-activedescendant', `ec-opt-${activeIndex}`);
  }

  function selectTaskFromEl(el) {
    st.task = {
      id: parseInt(el.dataset.taskId, 10),
      name: el.dataset.taskName || '',
      project_id: parseInt(el.dataset.projectId, 10) || 0,
      project_name: el.dataset.projectName || '',
    };
    st.projectIdOverride = null;
    // The recent_tasks table has no project_id column, so recent chips arrive
    // without one. Resolve it rather than making the user pick a project by
    // hand on the fastest path in the whole flow.
    st.resolvingProject = !st.task.project_id;
    renderTaskArea();
    renderChecks();
    renderFooter();
    schedulePreflight();
    q('#ec-duration')?.focus();
    if (st.resolvingProject) resolveTaskProject(st.task.id);
  }

  async function resolveTaskProject(taskId) {
    let t = null;
    try { t = await invoke('get_task_details', { taskId }); } catch (_) {}
    if (!open || !st.task || st.task.id !== taskId) return;
    if (t && t.project_id) {
      st.task.project_id = t.project_id;
      st.task.project_name = t.project_name || st.task.project_name;
    }
    // Either way we are done resolving; on failure the <select> is the fallback.
    st.resolvingProject = false;
    renderProjectLine();
    renderFooter();
    schedulePreflight();
  }

  function clearTask() {
    st.task = null;
    st.projectIdOverride = null;
    st.preflight = null;
    renderTaskArea();
    renderChecks();
    renderFooter();
    setTimeout(() => q('#ec-task-input')?.focus(), 0);
  }

  // ── Date row ────────────────────────────────────────────────────────

  function weekdaysOfViewedWeek() {
    const ref = new Date(st.date + 'T12:00:00');
    const monday = new Date(ref);
    monday.setDate(ref.getDate() - ((ref.getDay() + 6) % 7));
    const out = [];
    for (let i = 0; i < 5; i++) {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      out.push(`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`);
    }
    return out;
  }

  function renderDateRow() {
    const days = [];
    const today = todayDate();
    const seen = new Set();
    for (const d of [today, addDays(today, -1), ...weekdaysOfViewedWeek()]) {
      if (!seen.has(d)) { seen.add(d); days.push(d); }
    }
    q('#ec-date-row').innerHTML = days.map(d => {
      const total = st.dayTotals[d];
      // Each chip is annotated with that day's logged total, so an under-filled
      // day is visible without navigating away.
      const sub = total == null ? '' : (total > 0 ? formatHours(total) : '—');
      return `<button type="button" class="ec-date-chip${d === st.date ? ' active' : ''}" data-ec-date="${d}"
        aria-pressed="${d === st.date ? 'true' : 'false'}">
        <span class="ec-date-chip-day">${esc(dayLabel(d))}</span>
        <span class="ec-date-chip-sub">${esc(sub)}</span>
      </button>`;
    }).join('');
    const di = q('#ec-date');
    if (di && di.value !== st.date) di.value = st.date;
  }

  async function loadDayTotals() {
    const today = todayDate();
    const days = Array.from(new Set([today, addDays(today, -1), ...weekdaysOfViewedWeek()]));
    const results = await Promise.all(days.map(d =>
      invoke('get_entries_for_date', { date: d }).catch(() => null)
    ));
    days.forEach((d, i) => {
      if (results[i]) st.dayTotals[d] = results[i].reduce((s, e) => s + (e.hours || 0), 0);
    });
    if (open) renderDateRow();
  }

  // ── Duration chips + echo ───────────────────────────────────────────

  function renderQuickChips() {
    const base = [['15m', 0.25], ['30m', 0.5], ['1h', 1], ['2h', 2], ['4h', 4]];
    let html = base.map(([label, h]) =>
      `<button type="button" class="ec-quick-chip" data-ec-add="${h}">+${esc(label)}</button>`).join('');
    if (st.gapHours > 0.25) {
      html += `<button type="button" class="ec-quick-chip is-gap" data-ec-add="${st.gapHours}" data-ec-set="1"
        title="Fill the gap between attendance and logged time">${esc(formatHours(st.gapHours))} unaccounted</button>`;
    }
    q('#ec-duration-quick').innerHTML = html;
  }

  async function loadGap() {
    st.gapHours = 0;
    renderQuickChips();
    const cached = analysisCache.get(st.date);
    const fresh = cached && (Date.now() - cached.at) < ANALYSIS_TTL;
    try {
      const a = fresh ? cached.analysis : await invoke('get_day_analysis', { date: st.date });
      if (!fresh) analysisCache.set(st.date, { at: Date.now(), analysis: a });
      if (!open || !a) return;
      st.gapHours = Math.max(0, a.gap_hours || 0);
      renderQuickChips();
    } catch (_) {}
  }

  function renderDurationEcho() {
    const el = q('#ec-duration-echo');
    const r = parseDuration(st.durationText);
    if (r.empty) { el.textContent = ''; el.className = 'ec-duration-echo'; return; }
    if (!r.ok) { el.textContent = r.error; el.className = 'ec-duration-echo is-err'; return; }
    // The literal float that becomes unit_amount — the parse is pure and local,
    // so this updates on every keystroke with no debounce.
    el.textContent = `→ ${r.hours.toFixed(2)} h`;
    el.className = 'ec-duration-echo';
  }

  // ── Preflight ───────────────────────────────────────────────────────

  function formValid() {
    const r = parseDuration(st.durationText);
    return !!(st.task && st.task.id > 0 && currentProjectId() > 0 && st.date && r.ok);
  }

  function schedulePreflight() {
    clearTimeout(preflightTimer);
    if (!formValid()) { st.preflight = null; renderChecks(); return; }
    st.preflightBusy = true;
    renderChecks();
    preflightTimer = setTimeout(runPreflight, 250);
  }

  async function runPreflight() {
    const token = ++preflightToken;
    const r = parseDuration(st.durationText);
    if (!r.ok) return;
    try {
      const pf = await invoke('preflight_manual_entry', {
        taskId: st.task.id,
        projectId: currentProjectId(),
        durationHours: r.hours,
        date: st.date,
        excludeOdooId: st.odooId,
      });
      if (token !== preflightToken || !open) return;
      st.preflight = pf;
      st.preflightBusy = false;
      renderChecks();
      renderFooter();
      renderDestination();
    } catch (_) {
      if (token !== preflightToken) return;
      st.preflightBusy = false;
      renderChecks();
    }
  }

  /** True when an edit was started but Odoo is unreachable. Edits are NEVER
   *  queued: pending_timesheets has create semantics only, so
   *  update_timesheet_entry hard-errors offline. Say so up front rather than
   *  after the whole form has been filled in. */
  function offlineEditBlocked() {
    return st.mode === 'edit' && !!st.preflight && !st.preflight.online;
  }

  function renderDestination() {
    const el = q('#ec-destination');
    const pf = st.preflight;
    if (!pf) { el.textContent = ''; el.className = 'ec-destination'; return; }
    // Repair rewrites the queued row locally — no Odoo I/O either way.
    if (st.mode === 'repair') {
      el.textContent = 'Updates the queued entry';
      el.className = 'ec-destination is-offline';
    } else if (offlineEditBlocked()) {
      el.textContent = 'Offline — cannot edit';
      el.className = 'ec-destination is-offline';
    } else if (pf.online) {
      el.textContent = 'Writes to Odoo';
      el.className = 'ec-destination is-online';
    } else {
      el.textContent = 'Offline — will queue';
      el.className = 'ec-destination is-offline';
    }
  }

  // ── Checks strip ────────────────────────────────────────────────────

  function checkRow(state, text, extra = '') {
    const glyph = state === 'ok' ? '✓' : state === 'warn' ? '!' : '×';
    // Colour is always paired with a distinct glyph so the strip survives
    // [data-theme="colorblind"], where success→blue and danger→orange.
    return `<div class="ec-check ${state}">
      <span class="ec-check-dot" aria-hidden="true">${glyph}</span>
      <span class="ec-check-text">${text}</span>
      ${extra}
    </div>`;
  }

  function renderChecks() {
    const el = q('#ec-checks');
    const pf = st.preflight;

    if (st.rejected) { renderRejected(); return; }

    if (!formValid()) {
      el.className = 'ec-checks';
      el.innerHTML = `<div class="ec-check muted"><span class="ec-check-dot" aria-hidden="true">·</span>
        <span class="ec-check-text">Pick a task and a duration to run the checks.</span></div>`;
      return;
    }

    // While preflight is in flight keep the PREVIOUS values at 0.6 opacity
    // rather than blanking, so there is no layout jump.
    el.className = 'ec-checks' + (st.preflightBusy ? ' is-busy' : '');
    if (!pf) {
      el.innerHTML = `<div class="ec-check muted"><span class="sync-spinner" aria-hidden="true"></span>
        <span class="ec-check-text">Checking…</span></div>`;
      return;
    }

    const r = parseDuration(st.durationText);
    const newH = r.ok ? r.hours : 0;
    const dayTotal = pf.day_total_hours || 0;
    const sum = dayTotal + newH;

    let html = '';

    // Day total
    const dayState = sum > 24 ? 'err' : sum > 16 ? 'warn' : 'ok';
    html += checkRow(dayState,
      `${esc(formatHours(dayTotal))} logged + ${esc(formatHours(newH))} new = <strong>${esc(formatHours(sum))}</strong>`);

    // Duplicate
    const dups = (pf.duplicates || []);
    const dedupUnavailable = (pf.warnings || []).some(w => w.code === 'DEDUP_UNAVAILABLE');
    if (!pf.online) {
      html += checkRow('warn', "Can't check duplicates offline — checked on sync");
    } else if (dedupUnavailable) {
      html += checkRow('warn', "Couldn't reach Odoo to check for duplicates");
    } else if (dups.length && !st.allowDuplicate) {
      const d = dups[0];
      html += checkRow('warn',
        `Matching line already in Odoo`,
        `<div class="ec-dup">
           <div class="ec-dup-line">#${esc(d.odoo_id)} · ${esc(d.description || d.task_name)} · ${esc(formatHours(d.hours))}</div>
           <div class="ec-dup-actions">
             <button type="button" class="btn btn-sm btn-secondary" data-ec-dup="open" data-odoo-id="${escAttr(d.odoo_id)}"
               data-hours="${escAttr(d.hours)}" data-description="${escAttr(d.description || '')}">Open the existing one</button>
             <button type="button" class="btn btn-sm btn-secondary" data-ec-dup="anyway">Second block — log anyway</button>
           </div>
         </div>`);
    } else if (st.allowDuplicate) {
      html += checkRow('ok', 'Duplicate check acknowledged — will log anyway');
    } else {
      html += checkRow('ok', 'No matching line in Odoo');
    }

    // Running timer — names the live double-count trap. Never blocks: logging
    // this morning's forgotten hour on the task you are on now is legitimate.
    if (pf.timer_task_id && st.task && pf.timer_task_id === st.task.id) {
      html += checkRow('warn',
        `Timer is running on this task (${esc(formatClock(pf.timer_elapsed_secs || 0))} so far). Stopping it will log that separately.`);
    }

    // Connection — the copy must match what the backend will actually do.
    if (st.mode === 'repair') {
      html += checkRow('ok', 'Rewrites the queued entry — sent on the next sync');
    } else if (offlineEditBlocked()) {
      html += checkRow('err', 'Offline — reconnect to Odoo to edit this entry');
    } else if (pf.online) {
      html += checkRow('ok', 'Online — writes straight to Odoo');
    } else {
      html += checkRow('warn', 'Offline — will queue and sync later');
    }

    // Blank description substitution, stated rather than done magically.
    if (!st.description.trim() && st.task) {
      html += checkRow('ok', `Description will be “${esc(st.task.name)}”`);
    }

    // Advisory date warnings from the backend
    for (const w of (pf.warnings || [])) {
      if (w.code === 'FUTURE_DATE' || w.code === 'VERY_OLD_DATE') {
        html += checkRow('warn', esc(w.message));
      }
    }

    el.innerHTML = html;
  }

  function onChecksClick(e) {
    const dup = e.target.closest('[data-ec-dup]');
    if (dup) {
      if (dup.dataset.ecDup === 'anyway') {
        st.allowDuplicate = true;
        renderChecks();
        renderFooter();
        // renderChecks() destroyed the button that was just clicked; without
        // this, focus is on <body> and Tab leaves the dialog.
        q('#ec-submit')?.focus();
      } else {
        // Load the existing line into edit mode. Duplicates match on task_id,
        // so the currently-selected task/project are the right ones to carry.
        openComposer({
          mode: 'edit',
          odooId: parseInt(dup.dataset.odooId, 10),
          date: st.date,
          taskId: st.task.id,
          taskName: st.task.name,
          projectId: currentProjectId(),
          projectName: st.task.project_name,
          description: dup.dataset.description || '',
          durationHours: parseFloat(dup.dataset.hours),
        });
      }
      return;
    }

    const rec = e.target.closest('[data-ec-recover]');
    if (rec) handleRecovery(rec.dataset.ecRecover);
  }

  // ── Footer ──────────────────────────────────────────────────────────

  function submitLabel() {
    if (st.mode === 'edit') return 'Save changes';
    if (st.mode === 'repair') return 'Queue for retry';
    // The label NAMES THE DESTINATION — one word of copy that makes offline
    // behaviour impossible to misunderstand.
    if (st.preflight && !st.preflight.online) return 'Queue for later';
    return 'Create in Odoo';
  }

  function hardBlocked() {
    if (!formValid()) return true;
    if (offlineEditBlocked()) return true;
    const pf = st.preflight;
    if (!pf) return false;
    const r = parseDuration(st.durationText);
    return ((pf.day_total_hours || 0) + (r.ok ? r.hours : 0)) > 24;
  }

  function renderFooter() {
    if (st.rejected) return; // the rejection panel owns the footer area
    const el = q('#ec-footer');
    const disabled = hardBlocked() || st.submitting || (st.mode === 'edit' && !isDirty());
    el.className = 'ec-footer';
    el.innerHTML = `
      <button type="button" class="btn btn-secondary btn-sm" id="ec-cancel">Cancel</button>
      <div class="ec-footer-spacer"></div>
      ${st.mode === 'create'
        ? '<button type="button" class="btn btn-secondary btn-sm" id="ec-save-another">Save &amp; add another</button>'
        : ''}
      <button type="button" class="btn btn-primary btn-sm${st.submitting ? ' is-loading' : ''}" id="ec-submit"
        ${disabled ? 'disabled' : ''}>${esc(submitLabel())}</button>`;
    renderDestination();
  }

  function onFooterClick(e) {
    if (e.target.closest('#ec-cancel')) { requestClose(); return; }
    if (e.target.closest('#ec-save-another')) { submit(true); return; }
    if (e.target.closest('#ec-submit')) { submit(false); return; }
  }

  // ── Submit ──────────────────────────────────────────────────────────

  async function submit(andAnother) {
    if (st.submitting || hardBlocked()) return;
    const r = parseDuration(st.durationText);
    if (!r.ok) { renderDurationEcho(); return; }

    const desc = st.description.trim();
    const key = `${st.task.id}|${st.date}|${r.hours}|${desc}`;
    const now = Date.now();
    if (key === lastSubmit.key && (now - lastSubmit.at) < SUBMIT_GUARD_MS) return;
    lastSubmit = { key, at: now };

    // Synchronously before the await — the button cannot be double-fired.
    st.submitting = true;
    renderFooter();

    try {
      let res;
      if (st.mode === 'edit') {
        res = await invoke('update_timesheet_entry', {
          odooId: st.odooId,
          taskId: st.task.id,
          projectId: currentProjectId(),
          taskName: st.task.name,
          projectName: st.task.project_name || '',
          description: desc,
          durationHours: r.hours,
          date: st.date,
          originalDate: st.originalDate,
        });
      } else if (st.mode === 'repair') {
        await invoke('update_pending_entry', {
          entryId: st.pendingId,
          taskId: st.task.id,
          projectId: currentProjectId(),
          taskName: st.task.name,
          projectName: st.task.project_name || '',
          description: desc,
          durationHours: r.hours,
          date: st.date,
          allowDuplicate: st.allowDuplicate,
        });
        res = { outcome: 'queued' };
      } else {
        res = await invoke('create_manual_entry', {
          taskId: st.task.id,
          projectId: currentProjectId(),
          taskName: st.task.name,
          projectName: st.task.project_name || '',
          description: desc,
          durationHours: r.hours,
          date: st.date,
          allowDuplicate: st.allowDuplicate,
        });
      }

      st.submitting = false;
      handleOutcome(res, andAnother, r.hours);
    } catch (err) {
      st.submitting = false;
      lastSubmit = { key: '', at: 0 };
      // A thrown Err is a validation failure (a plain string from AppError).
      st.rejected = { message: prettifyOdooError(err), isPermanent: true, validation: true };
      renderChecks();
    }
  }

  function handleOutcome(res, andAnother, hours) {
    const outcome = res && res.outcome;

    if (outcome === 'needs_confirm') {
      // NOTHING WAS WRITTEN. The dialog stays open and the user chooses, so the
      // manual path can never leave a surprise status='duplicate' row behind.
      st.preflight = Object.assign({}, st.preflight || { online: true, day_total_hours: 0 }, {
        duplicates: res.duplicates || [],
      });
      lastSubmit = { key: '', at: 0 };
      renderChecks();
      renderFooter();
      return;
    }

    if (outcome === 'rejected') {
      st.rejected = { message: res.error || 'Odoo refused the entry.', isPermanent: !!res.is_permanent };
      lastSubmit = { key: '', at: 0 };
      renderChecks();
      return;
    }

    // created | queued | updated
    clearDraft();
    const date = st.date;
    const info = {
      outcome,
      date,
      entry: res && res.entry ? res.entry : null,
      odooId: res && res.odoo_id != null ? res.odoo_id : null,
      pendingId: res && res.pending_id != null ? res.pending_id : null,
      originalDate: st.originalDate,
    };

    if (outcome === 'queued') {
      showToast(`Queued ${formatHours(hours)} — will sync when Odoo is reachable`, 'warning');
    } else if (outcome === 'updated') {
      showToast('Entry updated', 'success');
    }
    // 'created' is deliberately SILENT. A toast on the 90% case is a tax paid on
    // every entry, and it is what trains people to ignore toasts when one
    // finally matters. The row flashes in the day list instead.

    if (typeof onChanged === 'function') onChanged(date, info);

    if (andAnother && st.mode === 'create') {
      // Keep date and project, clear task/duration/description.
      st.task = null;
      st.projectIdOverride = null;
      st.durationText = '';
      st.description = '';
      st.descBlank = true;
      st.allowDuplicate = false;
      st.preflight = null;
      st.initial = snapshot();
      q('#ec-duration').value = '';
      q('#ec-description').value = '';
      renderTaskArea();
      renderDurationEcho();
      renderChecks();
      renderFooter();
      loadDayTotals();
      setTimeout(() => q('#ec-task-input')?.focus(), 0);
      return;
    }

    closeComposer();
  }

  // ── Rejection recovery ──────────────────────────────────────────────

  function renderRejected() {
    const el = q('#ec-checks');
    el.className = 'ec-checks ec-rejected';
    const rej = st.rejected;
    el.innerHTML = `
      <div class="ec-error-panel" role="alert" aria-live="assertive">
        <div class="ec-error-title">${rej.validation ? 'Cannot save this entry' : 'Odoo refused this entry'}</div>
        <div class="sync-review-error ec-error-text">${esc(rej.message)}</div>
        <div class="ec-error-actions">
          ${rej.validation ? '' : `
            <button type="button" class="btn btn-sm btn-secondary" data-ec-recover="default">Use default task</button>
            <button type="button" class="btn btn-sm btn-secondary" data-ec-recover="pick">Pick a different task</button>
            <button type="button" class="btn btn-sm btn-secondary" data-ec-recover="queue">Queue anyway</button>`}
          <button type="button" class="btn btn-sm btn-secondary" data-ec-recover="back">Back to editing</button>
        </div>
      </div>`;
    const footer = q('#ec-footer');
    footer.className = 'ec-footer';
    footer.innerHTML = '<button type="button" class="btn btn-secondary btn-sm" id="ec-cancel">Close</button>';
  }

  async function handleRecovery(action) {
    if (action === 'back' || action === 'pick') {
      st.rejected = null;
      if (action === 'pick') clearTask();
      else { renderChecks(); renderFooter(); }
      return;
    }

    if (action === 'default') {
      try {
        const dt = await invoke('get_default_task');
        if (!dt || !dt.task_id) { showToast('No default task configured — set one in Settings', 'warning'); return; }
        const originalName = st.task ? st.task.name : '';
        st.task = {
          id: dt.task_id, name: dt.task_name || `Task #${dt.task_id}`,
          project_id: dt.project_id || 0, project_name: dt.project_name || '',
        };
        // Same semantics log_time already applies on the private-task fallback,
        // but shown as a reviewable change rather than done silently.
        st.description = `[${originalName}] ${st.description}`.trim();
        q('#ec-description').value = st.description;
        st.descBlank = !st.description.trim();
        st.rejected = null;
        renderTaskArea();
        renderChecks();
        renderFooter();
        schedulePreflight();
      } catch (err) {
        showToast(prettifyOdooError(err), 'error');
      }
      return;
    }

    if (action === 'queue') {
      const r = parseDuration(st.durationText);
      if (!r.ok || !st.task) return;
      try {
        // CONTRACT GAP: create_manual_entry writes NOTHING on an Odoo refusal,
        // and update_pending_entry can only update a row that already exists —
        // so the contract has no verb that enqueues a rejected entry. We fall
        // back to the existing log_time command, which queues on any Odoo error
        // (and applies the configured default-task redirect when there is one).
        // Either way the user's typing is persisted rather than lost.
        await invoke('log_time', {
          taskId: st.task.id,
          projectId: currentProjectId(),
          taskName: st.task.name,
          projectName: st.task.project_name || '',
          description: st.description.trim() || st.task.name,
          durationHours: r.hours,
          date: st.date,
        });
        showToast('Queued — will retry on the next sync', 'warning');
        st.rejected = null;
        clearDraft();
        if (typeof onChanged === 'function') onChanged(st.date, { outcome: 'queued', date: st.date });
        closeComposer();
      } catch (err) {
        showToast(prettifyOdooError(err), 'error');
      }
    }
  }

  // ── Keyboard inside the card ────────────────────────────────────────

  function onCardKeydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      requestClose();
      return;
    }

    // Ctrl/Cmd+Enter submits from anywhere, including the textarea.
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      submit(false);
      return;
    }

    const inCombo = e.target && e.target.id === 'ec-task-input';
    if (inCombo) {
      if (e.key === 'ArrowDown') { e.preventDefault(); moveActive(1); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); moveActive(-1); return; }
      if (e.key === 'Enter') {
        // Enter in the combobox selects the active result; it never submits.
        e.preventDefault();
        const items = root.querySelectorAll('#ec-results .ec-result');
        if (activeIndex >= 0 && items[activeIndex]) selectTaskFromEl(items[activeIndex]);
        return;
      }
    }

    // Plain Enter submits only from single-line fields, matching the
    // #new-task-modal rule that exempts its description textarea.
    if (e.key === 'Enter' && !e.shiftKey && e.target && e.target.tagName === 'INPUT' && !inCombo) {
      e.preventDefault();
      submit(false);
      return;
    }

    if (e.key === 'Tab') trapFocus(e);
  }

  function focusablesInCard() {
    const card = root.querySelector('.ec-card');
    return card ? card.querySelectorAll(
      'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    ) : [];
  }

  /** Tab pressed while focus sits on <body> — pull it back inside the card. */
  function onEscapedKeydown(e) {
    if (!open || !root || e.key !== 'Tab') return;
    if (e.target !== document.body) return;
    const focusables = focusablesInCard();
    if (!focusables.length) return;
    e.preventDefault();
    (e.shiftKey ? focusables[focusables.length - 1] : focusables[0]).focus();
  }

  function trapFocus(e) {
    const focusables = focusablesInCard();
    if (!focusables.length) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // ── Delete with undo ────────────────────────────────────────────────

  let undoTimer = null;

  function dismissUndo() {
    clearTimeout(undoTimer);
    document.getElementById('ec-undo-toast')?.remove();
  }

  function showUndoToast(message, onUndo) {
    dismissUndo();
    const el = document.createElement('div');
    el.id = 'ec-undo-toast';
    // The variant class lands this toast where that window's own toasts live —
    // dashboard toasts sit at the bottom (#dashboard-toast), tray at the top —
    // so the undo and its follow-up confirmation don't split across corners.
    el.className = `toast toast-warning ec-undo-toast is-${variant} visible`;
    el.setAttribute('role', 'status');
    el.innerHTML = `<span class="ec-undo-msg">${esc(message)}</span>
      <button type="button" class="btn-text ec-undo-btn">Undo</button>
      <span class="toast-countdown"></span>`;
    document.body.appendChild(el);
    el.querySelector('.ec-undo-btn').addEventListener('click', () => {
      dismissUndo();
      onUndo();
    });
    undoTimer = setTimeout(dismissUndo, UNDO_MS);
  }

  /**
   * Resolve the project an entry belongs to.
   *
   * Odoo reports an empty many2one as `false`, so `project_id` is legitimately
   * null on some lines and `entry.project_id || 0` would send project_id 0 —
   * which create_manual_entry rejects at the boundary. On the Undo path that
   * would fail AFTER the line has already been unlinked, so resolve it while
   * the entry still exists rather than at restore time.
   */
  async function resolveProjectId(entry) {
    if (entry.project_id) return entry.project_id;
    if (!entry.task_id) return 0;
    try {
      const t = await invoke('get_task_details', { taskId: entry.task_id });
      return (t && t.project_id) || 0;
    } catch (_) { return 0; }
  }

  /**
   * Delete an Odoo-backed entry. No confirmation dialog — a confirm on a
   * reversible action just trains people to click through confirms. The unlink
   * fires IMMEDIATELY (no soft-delete state to leak), with a 10-second undo.
   */
  async function deleteEntry(entry, rowEl) {
    const cap = entryCapabilities(entry);
    if (cap.kind !== 'odoo') return;

    const projectId = await resolveProjectId(entry);

    if (rowEl) rowEl.classList.add('removing');
    try {
      await invoke('delete_timesheet_entry', {
        odooId: entry.id,
        taskId: entry.task_id != null ? entry.task_id : null,
        date: entry.date,
      });
    } catch (err) {
      // Many Odoo configs block unlink on validated timesheets, so this is a
      // first-class path, not an edge case.
      if (rowEl) { rowEl.classList.remove('removing'); rowEl.classList.add('ec-shake'); setTimeout(() => rowEl.classList.remove('ec-shake'), 400); }
      showToast(prettifyOdooError(err), 'error');
      return;
    }

    if (typeof onChanged === 'function') onChanged(entry.date, { outcome: 'deleted', date: entry.date });

    showUndoToast(`Deleted ${formatHours(entry.hours)} on ${entry.task_name}`, async () => {
      if (!projectId) {
        // No project could be resolved and the backend rightly refuses
        // project_id 0. Hand the user the composer with everything else
        // prefilled instead of a dead-end error on an unrecoverable delete.
        showToast('Pick a project to restore this entry', 'warning');
        openComposer({
          mode: 'create',
          date: entry.date,
          taskId: entry.task_id,
          taskName: entry.task_name || '',
          projectName: entry.project_name || '',
          description: entry.description || '',
          durationHours: entry.hours,
          allowDuplicate: true,
        });
        return;
      }
      try {
        const res = await invoke('create_manual_entry', {
          taskId: entry.task_id,
          projectId,
          taskName: entry.task_name || '',
          projectName: entry.project_name || '',
          description: entry.description || '',
          durationHours: entry.hours,
          date: entry.date,
          // It would otherwise be flagged as a duplicate of itself.
          allowDuplicate: true,
        });
        if (res && res.outcome === 'created') {
          // The new Odoo id necessarily differs, so say so rather than
          // pretending identity was preserved.
          showToast(`Restored as #${res.odoo_id}`, 'success');
        } else if (res && res.outcome === 'queued') {
          showToast('Restored — queued for sync', 'warning');
        } else {
          showToast(prettifyOdooError((res && res.error) || 'Could not restore'), 'error');
        }
        if (typeof onChanged === 'function') onChanged(entry.date, { outcome: 'restored', date: entry.date });
      } catch (err) {
        showToast(prettifyOdooError(err), 'error');
      }
    });
  }

  /** Discard a queued row via the existing resolve_sync_entry('discard'). */
  async function deletePending(entry, rowEl) {
    if (rowEl) rowEl.classList.add('removing');
    try {
      await invoke('resolve_sync_entry', { entryId: entry.pending_id, action: 'discard' });
      showToast('Queued entry discarded', 'success');
      if (typeof onChanged === 'function') onChanged(entry.date, { outcome: 'deleted', date: entry.date });
    } catch (err) {
      if (rowEl) rowEl.classList.remove('removing');
      showToast(prettifyOdooError(err), 'error');
    }
  }

  // ── Public API ──────────────────────────────────────────────────────

  return {
    open: openComposer,
    close: closeComposer,
    // Escape-with-dirty-guard. Exposed so each window's document-level keydown
    // cascade can still close the composer when focus has escaped the card
    // (the card's own listener only sees events that originate inside it).
    requestClose,
    isOpen: () => open,
    deleteEntry,
    deletePending,
  };
}
