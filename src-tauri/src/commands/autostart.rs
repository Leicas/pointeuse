#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

use crate::error::{AppError, AppResult};

#[tauri::command]
pub async fn get_autostart_enabled(app_handle: tauri::AppHandle) -> AppResult<bool> {
    #[cfg(desktop)]
    {
        app_handle
            .autolaunch()
            .is_enabled()
            .map_err(|e| AppError::General(format!("Autostart check failed: {e}")))
    }
    #[cfg(mobile)]
    {
        let _ = app_handle;
        Ok(false)
    }
}

#[tauri::command]
pub async fn set_autostart_enabled(
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> AppResult<()> {
    #[cfg(desktop)]
    {
        let manager = app_handle.autolaunch();
        if enabled {
            manager
                .enable()
                .map_err(|e| AppError::General(format!("Failed to enable autostart: {e}")))
        } else {
            manager
                .disable()
                .map_err(|e| AppError::General(format!("Failed to disable autostart: {e}")))
        }
    }
    #[cfg(mobile)]
    {
        let _ = (enabled, app_handle);
        Err(AppError::General("Autostart is not supported on mobile".into()))
    }
}
