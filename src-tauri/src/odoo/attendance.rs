use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AttendanceStatus {
    pub is_checked_in: bool,
    pub attendance_id: Option<i64>,
    pub check_in_time: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceCheckInResult {
    pub attendance_id: i64,
    pub check_in_time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceCheckOutResult {
    pub worked_hours_today: f64,
}
