use crate::error::{AppError, AppResult};
use crate::odoo::attendance::{AttendanceCheckInResult, AttendanceCheckOutResult, AttendanceStatus};
use crate::state::AppState;

#[tauri::command]
pub async fn get_attendance_status(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceStatus> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    client.get_attendance_status().await
}

#[tauri::command]
pub async fn attendance_check_in(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceCheckInResult> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    let (attendance_id, check_in_time) = client.check_in().await?;

    Ok(AttendanceCheckInResult {
        attendance_id,
        check_in_time,
    })
}

#[tauri::command]
pub async fn attendance_check_out(
    state: tauri::State<'_, AppState>,
) -> AppResult<AttendanceCheckOutResult> {
    let client = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard
            .as_ref()
            .ok_or_else(|| AppError::Auth("Not logged in".into()))?
            .clone()
    };

    let worked_hours_today = client.check_out().await?;

    Ok(AttendanceCheckOutResult { worked_hours_today })
}
