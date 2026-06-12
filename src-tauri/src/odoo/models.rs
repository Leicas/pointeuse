use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use super::xmlrpc::XmlRpcValue;

// ---------------------------------------------------------------------------
// Model structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdooProject {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdooTask {
    pub id: i64,
    pub name: String,
    pub project_id: Option<(i64, String)>,
    pub stage_id: Option<(i64, String)>,
    pub stage_name: Option<String>,
    pub state: Option<String>,
    pub kanban_state: Option<String>,
    pub planned_hours: f64,
    pub effective_hours: f64,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub date_deadline: Option<String>,
    pub write_date: Option<String>,
    pub create_date: Option<String>,
    pub user_ids: Option<Vec<i64>>,
    pub parent_id: Option<(i64, String)>,
    pub color: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdooTimesheetEntry {
    pub id: i64,
    pub name: String,
    pub task_id: Option<(i64, String)>,
    pub project_id: Option<(i64, String)>,
    pub unit_amount: f64,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct OdooEmployee {
    pub id: i64,
    pub name: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a string field from an Odoo struct value, returning `""` if missing.
fn get_string(map: &std::collections::HashMap<String, XmlRpcValue>, key: &str) -> String {
    map.get(key)
        .and_then(|v| match v {
            XmlRpcValue::String(s) => Some(s.clone()),
            XmlRpcValue::Bool(false) | XmlRpcValue::Nil => Some(String::new()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Extract an optional string field (None for Odoo `false`/nil).
fn get_opt_string(map: &std::collections::HashMap<String, XmlRpcValue>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(XmlRpcValue::String(s)) => Some(s.clone()),
        Some(XmlRpcValue::Bool(false)) | Some(XmlRpcValue::Nil) | None => None,
        _ => None,
    }
}

/// Extract an i64 field (also handles doubles that Odoo sometimes returns).
fn get_i64(map: &std::collections::HashMap<String, XmlRpcValue>, key: &str) -> AppResult<i64> {
    map.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::Odoo(format!("Missing or invalid int field '{key}'")))
}

/// Extract a float field.
fn get_f64(map: &std::collections::HashMap<String, XmlRpcValue>, key: &str) -> f64 {
    map.get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

/// Extract a many2one `(id, name)` tuple. Returns `None` when Odoo sends `false`.
fn get_many2one(map: &std::collections::HashMap<String, XmlRpcValue>, key: &str) -> Option<(i64, String)> {
    map.get(key).and_then(|v| v.as_many2one())
}

// ---------------------------------------------------------------------------
// Conversions from XmlRpcValue
// ---------------------------------------------------------------------------

impl TryFrom<&XmlRpcValue> for OdooProject {
    type Error = AppError;

    fn try_from(val: &XmlRpcValue) -> AppResult<Self> {
        let map = val
            .as_struct()
            .ok_or_else(|| AppError::Odoo("Expected struct for OdooProject".into()))?;
        Ok(Self {
            id: get_i64(map, "id")?,
            name: get_string(map, "name"),
        })
    }
}

impl TryFrom<&XmlRpcValue> for OdooTask {
    type Error = AppError;

    fn try_from(val: &XmlRpcValue) -> AppResult<Self> {
        let map = val
            .as_struct()
            .ok_or_else(|| AppError::Odoo("Expected struct for OdooTask".into()))?;
        let stage = get_many2one(map, "stage_id");

        // Parse user_ids: Odoo many2many returns as array of integers
        let user_ids = match map.get("user_ids") {
            Some(XmlRpcValue::Array(arr)) => {
                let ids: Vec<i64> = arr.iter().filter_map(|v| v.as_i64()).collect();
                if ids.is_empty() { None } else { Some(ids) }
            }
            Some(XmlRpcValue::Bool(false)) | Some(XmlRpcValue::Nil) | None => None,
            _ => None,
        };

        // Parse color: optional integer
        let color = match map.get("color") {
            Some(v) => v.as_i64(),
            None => None,
        };

        Ok(Self {
            id: get_i64(map, "id")?,
            name: get_string(map, "name"),
            project_id: get_many2one(map, "project_id"),
            stage_id: stage.clone(),
            stage_name: stage.map(|(_, name)| name)
                .or_else(|| get_opt_string(map, "stage_id"))
                .or_else(|| get_opt_string(map, "stage_name")),
            state: get_opt_string(map, "state"),
            kanban_state: get_opt_string(map, "kanban_state"),
            planned_hours: get_f64(map, "planned_hours"),
            effective_hours: get_f64(map, "effective_hours"),
            description: get_opt_string(map, "description"),
            priority: get_opt_string(map, "priority"),
            date_deadline: get_opt_string(map, "date_deadline"),
            write_date: get_opt_string(map, "write_date"),
            create_date: get_opt_string(map, "create_date"),
            user_ids,
            parent_id: get_many2one(map, "parent_id"),
            color,
        })
    }
}

impl TryFrom<&XmlRpcValue> for OdooTimesheetEntry {
    type Error = AppError;

    fn try_from(val: &XmlRpcValue) -> AppResult<Self> {
        let map = val
            .as_struct()
            .ok_or_else(|| AppError::Odoo("Expected struct for OdooTimesheetEntry".into()))?;
        Ok(Self {
            id: get_i64(map, "id")?,
            name: get_string(map, "name"),
            task_id: get_many2one(map, "task_id"),
            project_id: get_many2one(map, "project_id"),
            unit_amount: get_f64(map, "unit_amount"),
            date: get_string(map, "date"),
        })
    }
}

impl TryFrom<&XmlRpcValue> for OdooEmployee {
    type Error = AppError;

    fn try_from(val: &XmlRpcValue) -> AppResult<Self> {
        let map = val
            .as_struct()
            .ok_or_else(|| AppError::Odoo("Expected struct for OdooEmployee".into()))?;
        Ok(Self {
            id: get_i64(map, "id")?,
            name: get_string(map, "name"),
        })
    }
}

/// Convert a Vec<XmlRpcValue> (from search_read) into typed models.
/// Logs and skips records that fail to parse instead of aborting.
pub fn parse_records<T>(records: Vec<XmlRpcValue>) -> AppResult<Vec<T>>
where
    T: for<'a> TryFrom<&'a XmlRpcValue, Error = AppError>,
{
    let total = records.len();
    let mut results = Vec::with_capacity(total);
    let mut errors = 0;
    for (i, rec) in records.iter().enumerate() {
        match T::try_from(rec) {
            Ok(val) => results.push(val),
            Err(e) => {
                errors += 1;
                if errors <= 3 {
                    log::error!("parse_records: failed to parse record {i}/{total}: {e}");
                    log::debug!("parse_records: raw record: {rec:?}");
                }
            }
        }
    }
    if errors > 3 {
        log::error!("parse_records: {errors} total parse failures out of {total} records");
    }
    Ok(results)
}
