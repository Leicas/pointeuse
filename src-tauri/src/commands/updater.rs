use serde::Serialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub body: Option<String>,
    pub date: Option<String>,
}

#[tauri::command]
pub async fn check_for_update(
    #[allow(unused_variables)] app: tauri::AppHandle,
) -> AppResult<UpdateInfo> {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        use tauri_plugin_updater::UpdaterExt;

        log::info!("check_for_update: checking...");

        let updater = app.updater_builder().build().map_err(|e| {
            AppError::General(format!("Failed to build updater: {e}"))
        })?;

        match updater.check().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                let body = update.body.clone();
                let date = update.date.map(|d| d.to_string());
                log::info!("check_for_update: update available v{version}");

                {
                    let state = app.state::<crate::state::AppState>();
                    let mut pending = state.pending_update.lock().unwrap();
                    *pending = Some(update);
                }

                Ok(UpdateInfo {
                    available: true,
                    version: Some(version),
                    body,
                    date,
                })
            }
            Ok(None) => {
                log::info!("check_for_update: no update available");
                Ok(UpdateInfo {
                    available: false,
                    version: None,
                    body: None,
                    date: None,
                })
            }
            Err(e) => {
                log::error!("check_for_update: error: {e}");
                Err(AppError::General(format!("Update check failed: {e}")))
            }
        }
    }

    #[cfg(mobile)]
    {
        Ok(UpdateInfo {
            available: false,
            version: None,
            body: None,
            date: None,
        })
    }
}

#[tauri::command]
pub async fn install_update(
    #[allow(unused_variables)] app: tauri::AppHandle,
) -> AppResult<()> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        log::info!("install_update: starting...");

        let update = {
            let state = app.state::<crate::state::AppState>();
            let mut pending = state.pending_update.lock().unwrap();
            pending.take()
        };

        let update = update.ok_or_else(|| {
            AppError::General("No pending update to install".into())
        })?;

        let mut downloaded = 0;
        update
            .download_and_install(
                |chunk_len, content_len| {
                    downloaded += chunk_len;
                    log::info!(
                        "install_update: downloaded {} / {}",
                        downloaded,
                        content_len.unwrap_or(0)
                    );
                },
                || {
                    log::info!("install_update: download complete, installing...");
                },
            )
            .await
            .map_err(|e| AppError::General(format!("Install failed: {e}")))?;

        log::info!("install_update: done, restart required");
        app.restart();
    }

    #[cfg(mobile)]
    {
        Err(AppError::General("Updates not available on mobile".into()))
    }
}
