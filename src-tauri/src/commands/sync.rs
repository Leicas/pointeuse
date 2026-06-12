use serde::Serialize;

use tauri::Emitter;

use crate::db::timesheets::{
    claim_entries_for_sync, cleanup_old_synced, get_entries_needing_review,
    get_pending_timesheets, get_sync_status_counts, mark_entry_duplicate, mark_entry_failed,
    mark_entry_synced, release_entry, resolve_entry, PendingTimesheet, SyncStatusCounts,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub synced: usize,
    pub failed: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub remaining: usize,
    pub needs_review: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub pending_count: usize,
    pub counts: SyncStatusCounts,
    pub needs_review: usize,
}

/// Tolerance for comparing hours (0.02h = ~1 minute).
const HOURS_TOLERANCE: f64 = 0.02;

#[tauri::command]
pub async fn sync_pending(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<SyncResult> {
    // Acquire sync lock — only one sync at a time
    {
        let mut lock = state.sync_in_progress.lock().unwrap();
        if *lock {
            log::info!("Sync already in progress, skipping");
            let db = state.db.lock().unwrap();
            let counts = get_sync_status_counts(&db)?;
            let review = get_entries_needing_review(&db)?;
            return Ok(SyncResult {
                synced: 0,
                failed: 0,
                duplicates: 0,
                rejected: 0,
                remaining: (counts.pending + counts.failed) as usize,
                needs_review: review.len(),
            });
        }
        *lock = true;
    }

    // Ensure we release the lock when done (even on early return)
    let result = do_sync(&app, &state).await;

    {
        let mut lock = state.sync_in_progress.lock().unwrap();
        *lock = false;
    }

    result
}

async fn do_sync(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
) -> AppResult<SyncResult> {
    log::info!("Starting sync of pending timesheets");

    let odoo_client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.clone()
    };

    let client = odoo_client.ok_or_else(|| AppError::Auth("Not authenticated".to_string()))?;

    // Step 1: Claim entries for sync (atomically sets status='syncing')
    let entries = {
        let db = state.db.lock().unwrap();
        claim_entries_for_sync(&db)?
    };

    if entries.is_empty() {
        // Clean up old synced entries
        let db = state.db.lock().unwrap();
        let _ = cleanup_old_synced(&db);
        let review = get_entries_needing_review(&db)?;
        return Ok(SyncResult {
            synced: 0,
            failed: 0,
            duplicates: 0,
            rejected: 0,
            remaining: 0,
            needs_review: review.len(),
        });
    }

    log::info!("Claimed {} entries for sync", entries.len());

    // Step 2: Batch-fetch existing Odoo timesheets for duplicate detection
    let unique_dates: Vec<String> = {
        let mut dates: Vec<String> = entries.iter().map(|e| e.date.clone()).collect();
        dates.sort();
        dates.dedup();
        dates
    };

    let existing_odoo_entries = match client.get_timesheets_for_dates(&unique_dates).await {
        Ok(entries) => {
            log::info!(
                "Fetched {} existing Odoo entries for duplicate check",
                entries.len()
            );
            entries
        }
        Err(e) => {
            log::warn!("Failed to fetch Odoo entries for dedup (proceeding without): {e}");
            Vec::new()
        }
    };

    // Step 3: Process each entry
    let mut synced = 0usize;
    let mut failed = 0usize;
    let mut duplicates = 0usize;
    let mut rejected = 0usize;

    for entry in &entries {
        // Check for duplicate in Odoo
        if let Some(matching) = find_duplicate(entry, &existing_odoo_entries) {
            log::warn!(
                "Entry {} (task={}, date={}, {:.2}h) matches existing Odoo entry id={}",
                entry.id,
                entry.task_id,
                entry.date,
                entry.duration_hours,
                matching.id
            );
            let db = state.db.lock().unwrap();
            if let Err(e) = mark_entry_duplicate(&db, entry.id, matching.id) {
                log::error!("Failed to mark entry {} as duplicate: {e}", entry.id);
                let _ = release_entry(&db, entry.id);
                failed += 1;
            } else {
                duplicates += 1;
                let _ = app.emit(
                    "sync_duplicate_found",
                    serde_json::json!({
                        "entry_id": entry.id,
                        "task_id": entry.task_id,
                        "date": entry.date,
                        "hours": entry.duration_hours,
                        "description": entry.description,
                        "matching_odoo_id": matching.id,
                        "matching_hours": matching.unit_amount,
                        "matching_description": matching.name,
                    }),
                );
            }
            continue;
        }

        // No duplicate found — send to Odoo
        match client
            .log_time(
                entry.task_id,
                entry.project_id,
                &entry.description,
                entry.duration_hours,
                &entry.date,
            )
            .await
        {
            Ok(odoo_id) => {
                let db = state.db.lock().unwrap();
                if let Err(e) = mark_entry_synced(&db, entry.id, odoo_id) {
                    log::error!(
                        "CRITICAL: Odoo accepted entry {} (odoo_id={}) but local mark failed: {e}. \
                         Entry stays in 'syncing' and will be caught as duplicate on next sync.",
                        entry.id,
                        odoo_id
                    );
                    failed += 1;
                } else {
                    synced += 1;
                    log::info!(
                        "Synced entry {} -> odoo_id={} (task={}, {:.2}h on {})",
                        entry.id,
                        odoo_id,
                        entry.task_id,
                        entry.duration_hours,
                        entry.date
                    );
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                let is_perm = is_permanent_error(&err_msg);
                let db = state.db.lock().unwrap();
                if let Err(db_err) = mark_entry_failed(&db, entry.id, &err_msg, is_perm) {
                    log::error!("Failed to mark entry {} as failed: {db_err}", entry.id);
                }
                if is_perm {
                    log::warn!(
                        "Entry {} permanently rejected by Odoo: {err_msg}",
                        entry.id
                    );
                    rejected += 1;
                    let _ = app.emit(
                        "sync_entry_rejected",
                        serde_json::json!({
                            "entry_id": entry.id,
                            "task_id": entry.task_id,
                            "date": entry.date,
                            "duration_hours": entry.duration_hours,
                            "description": entry.description,
                            "error": err_msg,
                        }),
                    );
                } else {
                    log::error!(
                        "Transient sync failure for entry {} (retry {}): {err_msg}",
                        entry.id,
                        entry.retry_count + 1
                    );
                    failed += 1;
                }
            }
        }
    }

    // Step 4: Clean up old synced entries and compute final status
    let db = state.db.lock().unwrap();
    let _ = cleanup_old_synced(&db);
    let review = get_entries_needing_review(&db)?;
    let counts = get_sync_status_counts(&db)?;
    let remaining = (counts.pending + counts.failed) as usize;

    log::info!(
        "Sync complete: {synced} synced, {duplicates} duplicates, {rejected} rejected, {failed} failed, {remaining} remaining, {} need review",
        review.len()
    );

    Ok(SyncResult {
        synced,
        failed,
        duplicates,
        rejected,
        remaining,
        needs_review: review.len(),
    })
}

/// Find a duplicate among existing Odoo entries.
/// Match criteria: same task_id, same date, hours within tolerance.
fn find_duplicate(
    entry: &PendingTimesheet,
    odoo_entries: &[crate::odoo::models::OdooTimesheetEntry],
) -> Option<crate::odoo::models::OdooTimesheetEntry> {
    for odoo in odoo_entries {
        let task_matches = odoo
            .task_id
            .as_ref()
            .is_some_and(|(id, _)| *id == entry.task_id);
        let date_matches = odoo.date == entry.date;
        let hours_match = (odoo.unit_amount - entry.duration_hours).abs() < HOURS_TOLERANCE;

        if task_matches && date_matches && hours_match {
            return Some(odoo.clone());
        }
    }
    None
}

/// Check whether an Odoo error is permanent (non-retryable).
fn is_permanent_error(error_msg: &str) -> bool {
    let msg = error_msg.to_lowercase();
    let permanent_indicators = [
        "private task",
        "access denied",
        "access error",
        "permission denied",
        "not allowed",
        "cannot be created",
        "forbidden",
        "access right",
        "access rights",
        "does not exist",
        "record not found",
        "validation error",
    ];
    permanent_indicators
        .iter()
        .any(|indicator| msg.contains(indicator))
}

#[tauri::command]
pub async fn get_sync_status(state: tauri::State<'_, AppState>) -> AppResult<SyncStatus> {
    let db = state.db.lock().unwrap();
    let counts = get_sync_status_counts(&db)?;
    let review = get_entries_needing_review(&db)?;
    let pending_count = (counts.pending + counts.failed + counts.syncing + counts.duplicate + counts.rejected) as usize;
    Ok(SyncStatus {
        pending_count,
        counts,
        needs_review: review.len(),
    })
}

#[tauri::command]
pub async fn get_pending_entries(
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<PendingTimesheet>> {
    let db = state.db.lock().unwrap();
    get_pending_timesheets(&db)
}

#[tauri::command]
pub async fn get_review_entries(
    state: tauri::State<'_, AppState>,
) -> AppResult<Vec<PendingTimesheet>> {
    let db = state.db.lock().unwrap();
    get_entries_needing_review(&db)
}

#[tauri::command]
pub async fn resolve_sync_entry(
    entry_id: i64,
    action: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!("resolve_sync_entry: id={entry_id}, action={action}");
    let db = state.db.lock().unwrap();
    resolve_entry(&db, entry_id, &action)
}

#[tauri::command]
pub async fn retry_sync_entry(
    entry_id: i64,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!("retry_sync_entry: resetting entry {entry_id} for retry");
    let db = state.db.lock().unwrap();
    resolve_entry(&db, entry_id, "force")
}
