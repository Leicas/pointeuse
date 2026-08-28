use crate::error::AppResult;

/// Relays an uncaught frontend (webview) error to Sentry. The frontend has no
/// build step, so instead of bundling a browser SDK it forwards through this
/// command and shares the Rust SDK's transport, release tagging, and offline
/// batching.
#[tauri::command]
pub async fn report_frontend_error(
    message: String,
    source: String,
    stack: String,
    window_label: String,
) -> AppResult<()> {
    log::error!("[frontend] {message} at {source} ({window_label})");
    sentry::with_scope(
        |scope| {
            scope.set_tag("side", "frontend");
            scope.set_tag("window", window_label);
            scope.set_extra("source", source.into());
            scope.set_extra("stack", stack.into());
        },
        || {
            sentry::capture_message(&message, sentry::Level::Error);
        },
    );
    Ok(())
}
