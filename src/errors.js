// Forwards uncaught frontend errors to the backend, which reports them to
// Sentry alongside Rust panics. Loaded as a plain (non-module) script before
// the window's main module so it also catches module-load failures.
(function () {
  const invoke = () => window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
  const seen = new Set();
  let sentThisSession = 0;

  function report(message, source, stack) {
    const call = invoke();
    if (!call) return;
    // Cap volume: one report per unique message, 20 per window lifetime.
    const key = String(message).slice(0, 200);
    if (seen.has(key) || sentThisSession >= 20) return;
    seen.add(key);
    sentThisSession += 1;
    call('report_frontend_error', {
      message: String(message).slice(0, 1000),
      source: String(source || '').slice(0, 300),
      stack: String(stack || '').slice(0, 4000),
      windowLabel: window.location.pathname,
    }).catch(() => {});
  }

  window.addEventListener('error', (event) => {
    const err = event.error;
    report(
      event.message || (err && err.message) || 'unknown error',
      `${event.filename || ''}:${event.lineno || 0}:${event.colno || 0}`,
      err && err.stack
    );
  });

  window.addEventListener('unhandledrejection', (event) => {
    const reason = event.reason;
    report(
      (reason && (reason.message || reason.toString())) || 'unhandled rejection',
      'unhandledrejection',
      reason && reason.stack
    );
  });
})();
