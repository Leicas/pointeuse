use std::collections::HashMap;

use chrono::Utc;

use crate::error::{AppError, AppResult};
use super::attendance::AttendanceStatus;
use super::models::{self, OdooProject, OdooTask};
use super::xmlrpc::{self, XmlRpcValue};

// ---------------------------------------------------------------------------
// OdooClient
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct OdooClient {
    http: reqwest::Client,
    url: String,
    database: String,
    username: String,
    password: String,
    uid: i64,
    employee_id: Option<i64>,
}

impl OdooClient {
    // -- accessors -----------------------------------------------------------

    pub fn url(&self) -> &str {
        &self.url
    }

    #[allow(dead_code)]
    pub fn database(&self) -> &str {
        &self.database
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn uid(&self) -> i64 {
        self.uid
    }

    #[allow(dead_code)]
    pub fn employee_id(&self) -> Option<i64> {
        self.employee_id
    }

    // -- constructor ---------------------------------------------------------

    /// Authenticate with Odoo via XML-RPC and return a connected client.
    pub async fn connect(
        url: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> AppResult<Self> {
        let url = url.to_string();
        let database = database.to_string();
        let username = username.to_string();
        let password = password.to_string();
        let http = reqwest::Client::new();

        // --- authenticate --------------------------------------------------
        let uid_val = xmlrpc::call_xmlrpc(
            &http,
            &url,
            "/xmlrpc/2/common",
            "authenticate",
            vec![
                XmlRpcValue::String(database.clone()),
                XmlRpcValue::String(username.clone()),
                XmlRpcValue::String(password.clone()),
                XmlRpcValue::Struct(HashMap::new()),
            ],
        )
        .await?;

        let uid = uid_val
            .as_i64()
            .ok_or_else(|| {
                // Odoo returns False (boolean 0) when credentials are wrong
                AppError::Auth("Authentication failed — check your credentials".into())
            })
            .and_then(|id| {
                if id <= 0 {
                    Err(AppError::Auth(
                        "Authentication failed — invalid uid returned".into(),
                    ))
                } else {
                    Ok(id)
                }
            })?;

        log::info!("Authenticated as uid={uid} on {url}");

        let mut client = Self {
            http,
            url,
            database,
            username,
            password,
            uid,
            employee_id: None,
        };

        // --- fetch employee_id ---------------------------------------------
        match client
            .search_read(
                "hr.employee",
                vec![XmlRpcValue::Array(vec![
                    XmlRpcValue::String("user_id".into()),
                    XmlRpcValue::String("=".into()),
                    XmlRpcValue::Int(uid),
                ])],
                vec!["id".into(), "name".into()],
                Some(1),
            )
            .await
        {
            Ok(records) if !records.is_empty() => {
                if let Some(map) = records[0].as_struct() {
                    if let Some(XmlRpcValue::Int(eid)) = map.get("id") {
                        client.employee_id = Some(*eid);
                        log::info!("Employee id: {eid}");
                    }
                }
            }
            Ok(_) => {
                log::warn!("No hr.employee record found for uid={uid}");
            }
            Err(e) => {
                log::warn!("Could not fetch employee: {e}");
            }
        }

        Ok(client)
    }

    // -----------------------------------------------------------------------
    // Low-level execute_kw
    // -----------------------------------------------------------------------

    async fn execute_kw(
        &self,
        model: &str,
        method: &str,
        args: Vec<XmlRpcValue>,
        kwargs: Option<XmlRpcValue>,
    ) -> AppResult<XmlRpcValue> {
        let kw = kwargs.unwrap_or_else(|| XmlRpcValue::Struct(HashMap::new()));

        xmlrpc::call_xmlrpc(
            &self.http,
            &self.url,
            "/xmlrpc/2/object",
            "execute_kw",
            vec![
                XmlRpcValue::String(self.database.clone()),
                XmlRpcValue::Int(self.uid),
                XmlRpcValue::String(self.password.clone()),
                XmlRpcValue::String(model.into()),
                XmlRpcValue::String(method.into()),
                XmlRpcValue::Array(args),
                kw,
            ],
        )
        .await
    }

    // -----------------------------------------------------------------------
    // CRUD helpers
    // -----------------------------------------------------------------------

    pub async fn search_read(
        &self,
        model: &str,
        domain: Vec<XmlRpcValue>,
        fields: Vec<String>,
        limit: Option<i64>,
    ) -> AppResult<Vec<XmlRpcValue>> {
        let mut kwargs = HashMap::new();

        // fields
        let fields_val: Vec<XmlRpcValue> = fields
            .into_iter()
            .map(XmlRpcValue::String)
            .collect();
        kwargs.insert("fields".into(), XmlRpcValue::Array(fields_val));

        if let Some(lim) = limit {
            kwargs.insert("limit".into(), XmlRpcValue::Int(lim));
        }

        log::info!("search_read: model={model}, limit={:?}", limit);

        let result = self
            .execute_kw(
                model,
                "search_read",
                vec![XmlRpcValue::Array(domain)],
                Some(XmlRpcValue::Struct(kwargs)),
            )
            .await?;

        match result {
            XmlRpcValue::Array(arr) => {
                log::info!("search_read: got {} records for {model}", arr.len());
                Ok(arr)
            }
            XmlRpcValue::Bool(false) => {
                log::info!("search_read: got False (empty) for {model}");
                Ok(Vec::new())
            }
            other => {
                log::error!("search_read: unexpected result type for {model}: {other:?}");
                Err(AppError::Odoo(format!(
                    "Unexpected search_read result: {other:?}"
                )))
            }
        }
    }

    pub async fn create(
        &self,
        model: &str,
        values: HashMap<String, XmlRpcValue>,
    ) -> AppResult<i64> {
        let result = self
            .execute_kw(
                model,
                "create",
                vec![XmlRpcValue::Struct(values)],
                None,
            )
            .await?;

        result
            .as_i64()
            .ok_or_else(|| AppError::Odoo(format!("create did not return an id: {result:?}")))
    }

    pub async fn write(
        &self,
        model: &str,
        ids: Vec<i64>,
        values: HashMap<String, XmlRpcValue>,
    ) -> AppResult<bool> {
        let id_vals: Vec<XmlRpcValue> = ids.into_iter().map(XmlRpcValue::Int).collect();

        let result = self
            .execute_kw(
                model,
                "write",
                vec![
                    XmlRpcValue::Array(id_vals),
                    XmlRpcValue::Struct(values),
                ],
                None,
            )
            .await?;

        Ok(result.as_bool().unwrap_or(false))
    }

    /// Delete records. Odoo returns `true` when the unlink succeeded.
    pub async fn unlink(&self, model: &str, ids: Vec<i64>) -> AppResult<bool> {
        let id_vals: Vec<XmlRpcValue> = ids.into_iter().map(XmlRpcValue::Int).collect();

        let result = self
            .execute_kw(model, "unlink", vec![XmlRpcValue::Array(id_vals)], None)
            .await?;

        Ok(result.as_bool().unwrap_or(false))
    }

    // -----------------------------------------------------------------------
    // Domain-specific methods
    // -----------------------------------------------------------------------

    fn task_fields() -> Vec<String> {
        vec![
            "id", "name", "project_id", "stage_id", "state",
            "description", "priority", "date_deadline",
            "write_date", "create_date", "user_ids",
            "parent_id", "color",
        ].into_iter().map(String::from).collect()
    }

    pub async fn get_projects(&self) -> AppResult<Vec<OdooProject>> {
        let records = self
            .search_read(
                "project.project",
                vec![],
                vec!["id".into(), "name".into()],
                None,
            )
            .await?;
        models::parse_records(records)
    }

    pub async fn search_tasks(
        &self,
        query: &str,
        project_id: Option<i64>,
    ) -> AppResult<Vec<OdooTask>> {
        let mut domain = Vec::new();

        if !query.is_empty() {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("name".into()),
                XmlRpcValue::String("ilike".into()),
                XmlRpcValue::String(query.into()),
            ]));
        }

        if let Some(pid) = project_id {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("project_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(pid),
            ]));
        }

        let records = self
            .search_read("project.task", domain, Self::task_fields(), Some(80))
            .await?;
        models::parse_records(records)
    }

    /// Fetch all tasks, optionally filtered by project_ids and/or user_ids.
    pub async fn get_all_tasks(
        &self,
        project_ids: &[i64],
        user_ids: &[i64],
    ) -> AppResult<Vec<OdooTask>> {
        let mut domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("state".into()),
                XmlRpcValue::String("not in".into()),
                XmlRpcValue::Array(vec![
                    XmlRpcValue::String("1_done".into()),
                    XmlRpcValue::String("1_canceled".into()),
                ]),
            ]),
        ];

        if !project_ids.is_empty() {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("project_id".into()),
                XmlRpcValue::String("in".into()),
                XmlRpcValue::Array(
                    project_ids.iter().map(|id| XmlRpcValue::Int(*id)).collect(),
                ),
            ]));
        }

        if !user_ids.is_empty() {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("user_ids".into()),
                XmlRpcValue::String("in".into()),
                XmlRpcValue::Array(
                    user_ids.iter().map(|id| XmlRpcValue::Int(*id)).collect(),
                ),
            ]));
        }

        let records = self
            .search_read("project.task", domain, Self::task_fields(), Some(500))
            .await?;
        models::parse_records(records)
    }

    /// Fetch internal users from res.users (non-share users).
    pub async fn get_all_users(&self) -> AppResult<Vec<(i64, String)>> {
        let domain = vec![XmlRpcValue::Array(vec![
            XmlRpcValue::String("share".into()),
            XmlRpcValue::String("=".into()),
            XmlRpcValue::Bool(false),
        ])];

        let records = self
            .search_read(
                "res.users",
                domain,
                vec!["id".into(), "name".into()],
                None,
            )
            .await?;

        let mut users = Vec::new();
        for r in &records {
            if let Some(m) = r.as_struct() {
                let id = m.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let name = m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id > 0 {
                    users.push((id, name));
                }
            }
        }
        users.sort_by_key(|a| a.1.to_lowercase());
        Ok(users)
    }

    pub async fn get_my_tasks(&self, uid: i64) -> AppResult<Vec<OdooTask>> {
        let domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("user_ids".into()),
                XmlRpcValue::String("in".into()),
                XmlRpcValue::Array(vec![XmlRpcValue::Int(uid)]),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("state".into()),
                XmlRpcValue::String("not in".into()),
                XmlRpcValue::Array(vec![
                    XmlRpcValue::String("1_done".into()),
                    XmlRpcValue::String("1_canceled".into()),
                ]),
            ]),
        ];

        let records = self
            .search_read("project.task", domain, Self::task_fields(), Some(200))
            .await?;
        models::parse_records(records)
    }

    pub async fn create_task(
        &self,
        name: &str,
        project_id: i64,
    ) -> AppResult<OdooTask> {
        let mut values = HashMap::new();
        values.insert("name".into(), XmlRpcValue::String(name.into()));
        values.insert("project_id".into(), XmlRpcValue::Int(project_id));

        let task_id = self.create("project.task", values).await?;

        // Read back the full record
        let records = self
            .search_read(
                "project.task",
                vec![XmlRpcValue::Array(vec![
                    XmlRpcValue::String("id".into()),
                    XmlRpcValue::String("=".into()),
                    XmlRpcValue::Int(task_id),
                ])],
                Self::task_fields(),
                Some(1),
            )
            .await?;

        let task_val = records
            .first()
            .ok_or_else(|| AppError::Odoo("Created task not found on re-read".into()))?;
        OdooTask::try_from(task_val)
    }

    /// Fetch available stages for a project from Odoo.
    /// Returns Vec<(stage_id, stage_name)>.
    pub async fn get_project_stages(
        &self,
        project_id: i64,
    ) -> AppResult<Vec<(i64, String)>> {
        let domain = vec![XmlRpcValue::Array(vec![
            XmlRpcValue::String("project_ids".into()),
            XmlRpcValue::String("in".into()),
            XmlRpcValue::Array(vec![XmlRpcValue::Int(project_id)]),
        ])];

        let records = self
            .search_read(
                "project.task.type",
                domain,
                vec!["id".into(), "name".into()],
                None,
            )
            .await?;

        let mut stages = Vec::new();
        for r in &records {
            if let Some(m) = r.as_struct() {
                let id = m.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let name = m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id > 0 {
                    stages.push((id, name));
                }
            }
        }
        Ok(stages)
    }

    /// Update the stage of a task.
    pub async fn update_task_stage(
        &self,
        task_id: i64,
        stage_id: i64,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert("stage_id".into(), XmlRpcValue::Int(stage_id));
        self.write("project.task", vec![task_id], values).await
    }

    /// Get the current stage, state, and kanban_state of a task.
    pub async fn get_task_full_state(
        &self,
        task_id: i64,
    ) -> AppResult<(Option<(i64, String)>, Option<String>, Option<String>)> {
        let records = self
            .search_read(
                "project.task",
                vec![XmlRpcValue::Array(vec![
                    XmlRpcValue::String("id".into()),
                    XmlRpcValue::String("=".into()),
                    XmlRpcValue::Int(task_id),
                ])],
                vec!["stage_id".into(), "state".into()],
                Some(1),
            )
            .await?;

        if let Some(r) = records.first() {
            if let Some(m) = r.as_struct() {
                let stage = m.get("stage_id").and_then(|v| v.as_many2one());
                let state = m.get("state").and_then(|v| v.as_str()).map(String::from);
                let kanban = m.get("kanban_state").and_then(|v| v.as_str()).map(String::from);
                return Ok((stage, state, kanban));
            }
        }
        Ok((None, None, None))
    }

    /// Update the state field of a task (e.g. 01_in_progress, 1_done).
    pub async fn update_task_state(
        &self,
        task_id: i64,
        state: &str,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert("state".into(), XmlRpcValue::String(state.into()));
        self.write("project.task", vec![task_id], values).await
    }

    /// Update the kanban_state of a task (normal/done/blocked).
    pub async fn update_task_kanban_state(
        &self,
        task_id: i64,
        kanban_state: &str,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert(
            "kanban_state".into(),
            XmlRpcValue::String(kanban_state.into()),
        );
        self.write("project.task", vec![task_id], values).await
    }

    /// Update the name of a task.
    pub async fn update_task_name(
        &self,
        task_id: i64,
        name: &str,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert("name".into(), XmlRpcValue::String(name.into()));
        self.write("project.task", vec![task_id], values).await
    }

    /// Update the description (HTML) of a task.
    pub async fn update_task_description(
        &self,
        task_id: i64,
        description: &str,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert("description".into(), XmlRpcValue::String(description.into()));
        self.write("project.task", vec![task_id], values).await
    }

    /// Update the deadline of a task. Pass None to clear.
    pub async fn update_task_deadline(
        &self,
        task_id: i64,
        date_deadline: Option<&str>,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        let val = match date_deadline {
            Some(d) => XmlRpcValue::String(d.into()),
            None => XmlRpcValue::Bool(false),
        };
        values.insert("date_deadline".into(), val);
        self.write("project.task", vec![task_id], values).await
    }

    /// Update the priority of a task ("0" = normal, "1" = urgent).
    pub async fn update_task_priority(
        &self,
        task_id: i64,
        priority: &str,
    ) -> AppResult<bool> {
        let mut values = HashMap::new();
        values.insert("priority".into(), XmlRpcValue::String(priority.into()));
        self.write("project.task", vec![task_id], values).await
    }

    /// Get full task details (all fields) for a single task.
    pub async fn get_task_details(
        &self,
        task_id: i64,
    ) -> AppResult<OdooTask> {
        let records = self
            .search_read(
                "project.task",
                vec![XmlRpcValue::Array(vec![
                    XmlRpcValue::String("id".into()),
                    XmlRpcValue::String("=".into()),
                    XmlRpcValue::Int(task_id),
                ])],
                Self::task_fields(),
                Some(1),
            )
            .await?;

        let task_val = records
            .first()
            .ok_or_else(|| AppError::Odoo(format!("Task {task_id} not found")))?;
        OdooTask::try_from(task_val)
    }

    /// Log time on a task. Creates an `account.analytic.line` record and then
    /// writes `{}` to the task to trigger Odoo's recompute of effective_hours.
    pub async fn log_time(
        &self,
        task_id: i64,
        project_id: i64,
        description: &str,
        hours: f64,
        date: &str,
    ) -> AppResult<i64> {
        let mut values = HashMap::new();
        values.insert("name".into(), XmlRpcValue::String(description.into()));
        values.insert("task_id".into(), XmlRpcValue::Int(task_id));
        values.insert("project_id".into(), XmlRpcValue::Int(project_id));
        values.insert("unit_amount".into(), XmlRpcValue::Double(hours));
        values.insert("date".into(), XmlRpcValue::String(date.into()));

        if let Some(eid) = self.employee_id {
            values.insert("employee_id".into(), XmlRpcValue::Int(eid));
        }

        let line_id = self.create("account.analytic.line", values).await?;

        // Write empty dict to task to trigger recompute
        self.write("project.task", vec![task_id], HashMap::new())
            .await?;

        log::info!("Logged {hours}h on task {task_id} (line_id={line_id})");
        Ok(line_id)
    }

    /// Create a timesheet line for the manual-entry path.
    ///
    /// Same value map as `log_time`, but the `project.task` recompute that follows
    /// is treated as a best-effort step: the line already exists at that point, so
    /// reporting the recompute failure as an error would make the caller retry and
    /// create a genuine duplicate in Odoo.
    pub async fn create_timesheet_line(
        &self,
        task_id: i64,
        project_id: i64,
        description: &str,
        hours: f64,
        date: &str,
    ) -> AppResult<i64> {
        let mut values = HashMap::new();
        values.insert("name".into(), XmlRpcValue::String(description.into()));
        values.insert("task_id".into(), XmlRpcValue::Int(task_id));
        values.insert("project_id".into(), XmlRpcValue::Int(project_id));
        values.insert("unit_amount".into(), XmlRpcValue::Double(hours));
        values.insert("date".into(), XmlRpcValue::String(date.into()));

        if let Some(eid) = self.employee_id {
            values.insert("employee_id".into(), XmlRpcValue::Int(eid));
        }

        let line_id = self.create("account.analytic.line", values).await?;

        self.recompute_task(task_id).await;

        log::info!("Created timesheet line {line_id} ({hours}h on task {task_id})");
        Ok(line_id)
    }

    /// Update an existing `account.analytic.line`.
    pub async fn update_timesheet_line(
        &self,
        line_id: i64,
        values: HashMap<String, XmlRpcValue>,
    ) -> AppResult<bool> {
        self.write("account.analytic.line", vec![line_id], values)
            .await
    }

    /// Write an empty dict to a task so Odoo recomputes `effective_hours`.
    /// Never fatal — a stale progress bar is not worth failing a write that landed.
    pub async fn recompute_task(&self, task_id: i64) {
        if let Err(e) = self.write("project.task", vec![task_id], HashMap::new()).await {
            log::warn!("recompute_task: effective_hours recompute failed for task {task_id}: {e}");
        }
    }

    // -----------------------------------------------------------------------
    // Attendance methods
    // -----------------------------------------------------------------------

    /// Check if the current employee is checked in.
    pub async fn get_attendance_status(&self) -> AppResult<AttendanceStatus> {
        let employee_id = self
            .employee_id
            .ok_or_else(|| AppError::Odoo("No employee record linked to this user".into()))?;

        let domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(employee_id),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("check_out".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Bool(false),
            ]),
        ];

        let records = self
            .search_read(
                "hr.attendance",
                domain,
                vec!["id".into(), "check_in".into()],
                Some(1),
            )
            .await?;

        if records.is_empty() {
            return Ok(AttendanceStatus {
                is_checked_in: false,
                attendance_id: None,
                check_in_time: None,
            });
        }

        let map = records[0]
            .as_struct()
            .ok_or_else(|| AppError::Odoo("Expected struct for attendance record".into()))?;

        let id = map
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| AppError::Odoo("Missing id in attendance record".into()))?;

        let check_in = map
            .get("check_in")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(AttendanceStatus {
            is_checked_in: true,
            attendance_id: Some(id),
            check_in_time: Some(check_in),
        })
    }

    /// Check in the current employee. Returns (attendance_id, check_in_time).
    pub async fn check_in(&self) -> AppResult<(i64, String)> {
        let employee_id = self
            .employee_id
            .ok_or_else(|| AppError::Odoo("No employee record linked to this user".into()))?;

        // Verify not already checked in
        let status = self.get_attendance_status().await?;
        if status.is_checked_in {
            return Err(AppError::Odoo("Already checked in".into()));
        }

        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut values = HashMap::new();
        values.insert("employee_id".into(), XmlRpcValue::Int(employee_id));
        values.insert("check_in".into(), XmlRpcValue::String(now.clone()));

        let attendance_id = self.create("hr.attendance", values).await?;

        log::info!("Checked in: attendance_id={attendance_id}, time={now}");
        Ok((attendance_id, now))
    }

    /// Check out the current employee. Returns today's total worked hours.
    pub async fn check_out(&self) -> AppResult<f64> {
        let employee_id = self
            .employee_id
            .ok_or_else(|| AppError::Odoo("No employee record linked to this user".into()))?;

        // Verify currently checked in
        let status = self.get_attendance_status().await?;
        let attendance_id = status
            .attendance_id
            .ok_or_else(|| AppError::Odoo("Not checked in".into()))?;

        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut values = HashMap::new();
        values.insert("check_out".into(), XmlRpcValue::String(now.clone()));

        self.write("hr.attendance", vec![attendance_id], values)
            .await?;

        log::info!("Checked out: attendance_id={attendance_id}, time={now}");

        // Fetch today's total worked hours
        let today_start = Utc::now().format("%Y-%m-%d 00:00:00").to_string();
        let domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(employee_id),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("check_in".into()),
                XmlRpcValue::String(">=".into()),
                XmlRpcValue::String(today_start),
            ]),
        ];

        let records = self
            .search_read(
                "hr.attendance",
                domain,
                vec!["worked_hours".into()],
                None,
            )
            .await?;

        let total_hours: f64 = records
            .iter()
            .filter_map(|r| r.as_struct())
            .filter_map(|m| m.get("worked_hours"))
            .filter_map(|v| v.as_f64())
            .sum();

        log::info!("Today's total worked hours: {total_hours:.2}");
        Ok(total_hours)
    }

    /// Fetch today's attendance records for the current employee.
    /// Returns a list of (check_in, check_out, worked_hours) tuples.
    pub async fn get_today_attendance(
        &self,
        date: &str,
    ) -> AppResult<Vec<(String, Option<String>, f64)>> {
        let employee_id = self
            .employee_id
            .ok_or_else(|| AppError::Odoo("No employee record linked to this user".into()))?;

        let day_start = format!("{date} 00:00:00");
        let day_end = format!("{date} 23:59:59");

        let domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(employee_id),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("check_in".into()),
                XmlRpcValue::String(">=".into()),
                XmlRpcValue::String(day_start),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("check_in".into()),
                XmlRpcValue::String("<=".into()),
                XmlRpcValue::String(day_end),
            ]),
        ];

        let records = self
            .search_read(
                "hr.attendance",
                domain,
                vec![
                    "check_in".into(),
                    "check_out".into(),
                    "worked_hours".into(),
                ],
                None,
            )
            .await?;

        let mut result = Vec::new();
        for r in &records {
            if let Some(m) = r.as_struct() {
                let check_in = m
                    .get("check_in")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let check_out = m
                    .get("check_out")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let worked = m
                    .get("worked_hours")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                result.push((check_in, check_out, worked));
            }
        }
        Ok(result)
    }

    /// Fetch timesheet entries for a date range from Odoo.
    pub async fn get_timesheets_for_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<models::OdooTimesheetEntry>> {
        log::info!("get_timesheets_for_range: {start_date} to {end_date}");
        let mut domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("date".into()),
                XmlRpcValue::String(">=".into()),
                XmlRpcValue::String(start_date.into()),
            ]),
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("date".into()),
                XmlRpcValue::String("<=".into()),
                XmlRpcValue::String(end_date.into()),
            ]),
        ];

        if let Some(eid) = self.employee_id {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(eid),
            ]));
        } else {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("user_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(self.uid),
            ]));
        }

        let records = self
            .search_read(
                "account.analytic.line",
                domain,
                vec![
                    "id".into(),
                    "name".into(),
                    "task_id".into(),
                    "project_id".into(),
                    "unit_amount".into(),
                    "date".into(),
                ],
                None,
            )
            .await?;
        models::parse_records(records)
    }

    /// Search for existing timesheets matching a set of dates for duplicate detection.
    /// Returns all user timesheets for the given dates so callers can match locally.
    pub async fn get_timesheets_for_dates(
        &self,
        dates: &[String],
    ) -> AppResult<Vec<models::OdooTimesheetEntry>> {
        if dates.is_empty() {
            return Ok(Vec::new());
        }

        log::info!("get_timesheets_for_dates: checking {} dates for duplicates", dates.len());

        let date_values: Vec<XmlRpcValue> = dates
            .iter()
            .map(|d| XmlRpcValue::String(d.clone()))
            .collect();

        let mut domain = vec![XmlRpcValue::Array(vec![
            XmlRpcValue::String("date".into()),
            XmlRpcValue::String("in".into()),
            XmlRpcValue::Array(date_values),
        ])];

        if let Some(eid) = self.employee_id {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(eid),
            ]));
        } else {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("user_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(self.uid),
            ]));
        }

        let records = self
            .search_read(
                "account.analytic.line",
                domain,
                vec![
                    "id".into(),
                    "name".into(),
                    "task_id".into(),
                    "project_id".into(),
                    "unit_amount".into(),
                    "date".into(),
                ],
                None,
            )
            .await?;
        models::parse_records(records)
    }

    /// Fetch today's timesheet entries from Odoo for the current user.
    pub async fn get_today_timesheets(&self, date: &str) -> AppResult<Vec<models::OdooTimesheetEntry>> {
        log::info!(
            "get_today_timesheets: date={date}, employee_id={:?}, uid={}",
            self.employee_id, self.uid
        );
        let mut domain = vec![
            XmlRpcValue::Array(vec![
                XmlRpcValue::String("date".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::String(date.into()),
            ]),
        ];

        if let Some(eid) = self.employee_id {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("employee_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(eid),
            ]));
        } else {
            domain.push(XmlRpcValue::Array(vec![
                XmlRpcValue::String("user_id".into()),
                XmlRpcValue::String("=".into()),
                XmlRpcValue::Int(self.uid),
            ]));
        }

        let records = self
            .search_read(
                "account.analytic.line",
                domain,
                vec![
                    "id".into(),
                    "name".into(),
                    "task_id".into(),
                    "project_id".into(),
                    "unit_amount".into(),
                    "date".into(),
                ],
                None,
            )
            .await?;
        models::parse_records(records)
    }
}
