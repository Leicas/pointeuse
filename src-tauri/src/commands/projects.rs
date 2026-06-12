use serde::Serialize;

use crate::db::projects::{cache_projects, get_cached_projects, ProjectLike};
use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub name: String,
}

impl ProjectLike for ProjectInfo {
    fn project_id(&self) -> i64 { self.id }
    fn project_name_ref(&self) -> &str { &self.name }
}

#[tauri::command]
pub async fn get_projects(state: tauri::State<'_, AppState>) -> AppResult<Vec<ProjectInfo>> {
    log::info!("Fetching projects");

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    if let Some(client) = odoo_client {
        match client.get_projects().await {
            Ok(projects) => {
                let project_infos: Vec<ProjectInfo> = projects
                    .into_iter()
                    .map(|p| ProjectInfo {
                        id: p.id,
                        name: p.name,
                    })
                    .collect();

                if let Ok(db) = state.db.lock() {
                    if let Err(e) = cache_projects(&db, &project_infos) {
                        log::error!("Failed to cache projects: {e}");
                    }
                }

                return Ok(project_infos);
            }
            Err(e) => {
                log::error!("Odoo get_projects failed, falling back to cache: {e}");
            }
        }
    }

    let db = state.db.lock().unwrap();
    let cached = get_cached_projects(&db)?;
    Ok(cached.into_iter().map(|c| ProjectInfo { id: c.id, name: c.name }).collect())
}
