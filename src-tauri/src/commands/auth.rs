use serde::Serialize;
use tauri::Manager;
use tauri_plugin_store::StoreExt;

#[cfg(desktop)]
use crate::credentials::load_credentials;
use crate::credentials::{clear_credentials, save_credentials};
use crate::error::{AppError, AppResult};
use crate::odoo::client::OdooClient;
use crate::odoo::xmlrpc::{self, XmlRpcValue};
use crate::state::AppState;

const STORE_FILE: &str = "settings.json";
const KEY_URL: &str = "odoo_url";
const KEY_DATABASE: &str = "odoo_database";
const KEY_USERNAME: &str = "odoo_username";
#[cfg(mobile)]
const KEY_PASSWORD: &str = "odoo_password";

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub uid: i64,
    pub username: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub username: Option<String>,
    pub url: Option<String>,
}

/// Resolve the database name from the URL.
/// Strategy:
/// 1. Try db.list (works on self-hosted, blocked on Odoo SaaS)
/// 2. For *.odoo.com, probe /web/database/selector or /web/session/get_session_info
/// 3. Fallback: try the subdomain
async fn resolve_database(url: &str) -> AppResult<String> {
    let subdomain = extract_subdomain(url);

    // Try db.list first (works on self-hosted)
    if let Ok(list) = detect_database_list(url).await {
        if list.len() == 1 {
            return Ok(list.into_iter().next().unwrap());
        }
        if !list.is_empty() {
            // Match by subdomain
            if let Some(matched) = list.iter().find(|db| {
                db.to_lowercase().contains(&subdomain.to_lowercase())
            }) {
                return Ok(matched.clone());
            }
            return Ok(list[0].clone());
        }
    }

    // For Odoo SaaS: probe /web/database/selector which returns the DB name
    if let Some(db) = probe_odoo_saas_database(url).await {
        return Ok(db);
    }

    // Final fallback: use subdomain
    if !subdomain.is_empty() {
        log::info!("Falling back to subdomain as database: {subdomain}");
        return Ok(subdomain);
    }

    Err(AppError::Auth("Could not detect database. Please enter it manually.".into()))
}

fn extract_subdomain(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .and_then(|host| {
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() >= 2 { Some(parts[0].to_string()) } else { None }
        })
        .unwrap_or_default()
}

/// Odoo SaaS: GET /web/session/get_session_info or /web returns a page
/// with the database name. We can also try POST /web/session/authenticate
/// with a wrong password — the error reveals the DB name. But the simplest
/// approach: Odoo's /web/database/list JSON endpoint.
async fn probe_odoo_saas_database(url: &str) -> Option<String> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    // Try the JSON-RPC endpoint that lists databases
    let json_body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "call",
        "id": 1,
        "params": {}
    });

    let resp = http
        .post(format!("{}/web/database/list", url.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .json(&json_body)
        .send()
        .await
        .ok()?;

    let text = resp.text().await.ok()?;
    log::debug!("Database list probe response: {}", &text[..text.len().min(500)]);

    // Parse JSON-RPC response: {"jsonrpc":"2.0","id":1,"result":["dbname"]}
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let result = parsed.get("result")?;

    if let Some(arr) = result.as_array() {
        let dbs: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        if dbs.len() == 1 {
            log::info!("Probed database via JSON-RPC: {}", dbs[0]);
            return Some(dbs[0].clone());
        }
        if !dbs.is_empty() {
            let subdomain = extract_subdomain(url);
            if let Some(matched) = dbs.iter().find(|db| db.to_lowercase().contains(&subdomain.to_lowercase())) {
                log::info!("Probed database via JSON-RPC (matched subdomain): {matched}");
                return Some(matched.clone());
            }
            log::info!("Probed database via JSON-RPC (first of {}): {}", dbs.len(), dbs[0]);
            return Some(dbs[0].clone());
        }
    }

    None
}

async fn detect_database_list(url: &str) -> AppResult<Vec<String>> {
    let http = reqwest::Client::new();
    match xmlrpc::call_xmlrpc(&http, url, "/xmlrpc/2/db", "list", vec![]).await {
        Ok(XmlRpcValue::Array(arr)) => {
            Ok(arr.into_iter().filter_map(|v| v.as_str().map(String::from)).collect())
        }
        Ok(_) => Ok(vec![]),
        Err(e) => {
            log::warn!("Could not list databases: {e}");
            Err(e)
        }
    }
}

/// Probe the Odoo instance to find available databases.
#[tauri::command]
pub async fn detect_database(url: String) -> AppResult<Vec<String>> {
    let url = url.trim_end_matches('/').to_string();
    detect_database_list(&url).await.or_else(|_| Ok(vec![]))
}

#[tauri::command]
pub async fn login(
    url: String,
    database: String,
    username: String,
    password: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<LoginResponse> {
    let url = url.trim_end_matches('/').to_string();

    // Auto-detect database if not provided
    let database = if database.is_empty() {
        resolve_database(&url).await?
    } else {
        database
    };

    log::info!("Attempting login for '{}' at {} (db: {})", username, url, database);

    let client = OdooClient::connect(&url, &database, &username, &password)
        .await
        .map_err(|e| AppError::Auth(format!("Login failed: {e}")))?;

    let uid = client.uid();

    // Save credentials: keyring for password (desktop), store for non-secret info
    save_credentials(&username, &password)
        .map_err(|e| AppError::Keyring(format!("Failed to save credentials: {e}")))?;

    // Save connection info to Tauri store for auto-reconnect
    match app_handle.store(STORE_FILE) {
        Ok(store) => {
            store.set(KEY_URL, serde_json::json!(&url));
            store.set(KEY_DATABASE, serde_json::json!(&database));
            store.set(KEY_USERNAME, serde_json::json!(&username));
            // On mobile, store password in Tauri store (no keyring available)
            #[cfg(mobile)]
            store.set(KEY_PASSWORD, serde_json::json!(&password));
            match store.save() {
                Ok(_) => log::info!("login: saved credentials to store"),
                Err(e) => log::error!("login: failed to save store: {e}"),
            }
        }
        Err(e) => log::error!("login: could not open store: {e}"),
    }

    {
        let mut odoo_guard = state.odoo.lock().unwrap();
        *odoo_guard = Some(client);
    }

    log::info!("Login successful for '{}' (uid={})", username, uid);

    Ok(LoginResponse { uid, username })
}

#[tauri::command]
pub async fn logout(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    log::info!("Logging out");

    let username = {
        let odoo_guard = state.odoo.lock().unwrap();
        odoo_guard.as_ref().map(|c| c.username().to_string())
    };

    if let Some(ref username) = username {
        if let Err(e) = clear_credentials(username) {
            log::error!("Failed to clear credentials from keyring: {e}");
        }
    }

    // Clear stored connection info
    if let Ok(store) = app_handle.store(STORE_FILE) {
        let _ = store.delete(KEY_URL);
        let _ = store.delete(KEY_DATABASE);
        let _ = store.delete(KEY_USERNAME);
        #[cfg(mobile)]
        let _ = store.delete(KEY_PASSWORD);
        let _ = store.save();
    }

    {
        let mut odoo_guard = state.odoo.lock().unwrap();
        *odoo_guard = None;
    }

    log::info!("Logout complete");
    Ok(())
}

#[tauri::command]
pub async fn check_auth(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<AuthStatus> {
    // Check if already connected
    {
        let odoo_guard = state.odoo.lock().unwrap();
        if let Some(client) = odoo_guard.as_ref() {
            return Ok(AuthStatus {
                authenticated: true,
                username: Some(client.username().to_string()),
                url: Some(client.url().to_string()),
            });
        }
    }

    // Not connected — try auto-login from saved credentials
    log::info!("check_auth: no active session, attempting auto-login");
    if try_auto_login(&app_handle).await.is_some() {
        let odoo_guard = state.odoo.lock().unwrap();
        if let Some(client) = odoo_guard.as_ref() {
            return Ok(AuthStatus {
                authenticated: true,
                username: Some(client.username().to_string()),
                url: Some(client.url().to_string()),
            });
        }
    }

    Ok(AuthStatus {
        authenticated: false,
        username: None,
        url: None,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedConnection {
    pub url: Option<String>,
    pub username: Option<String>,
}

/// Return saved connection info for prepopulating the login form.
#[tauri::command]
pub async fn get_saved_connection(app_handle: tauri::AppHandle) -> AppResult<SavedConnection> {
    if let Ok(store) = app_handle.store(STORE_FILE) {
        let url = store.get(KEY_URL).and_then(|v| v.as_str().map(String::from));
        let username = store.get(KEY_USERNAME).and_then(|v| v.as_str().map(String::from));
        Ok(SavedConnection { url, username })
    } else {
        Ok(SavedConnection { url: None, username: None })
    }
}

/// Try to auto-reconnect using saved credentials. Called from check_auth.
pub async fn try_auto_login(app_handle: &tauri::AppHandle) -> Option<()> {
    let store = match app_handle.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("auto_login: could not open store: {e}");
            return None;
        }
    };

    let url = match store.get(KEY_URL) {
        Some(v) => match v.as_str() {
            Some(s) => s.to_string(),
            None => { log::warn!("auto_login: url is not a string: {v:?}"); return None; }
        },
        None => { log::info!("auto_login: no saved url in store"); return None; }
    };

    let database = match store.get(KEY_DATABASE) {
        Some(v) => v.as_str().unwrap_or("").to_string(),
        None => { log::info!("auto_login: no saved database in store"); return None; }
    };

    let username = match store.get(KEY_USERNAME) {
        Some(v) => match v.as_str() {
            Some(s) => s.to_string(),
            None => { log::warn!("auto_login: username is not a string"); return None; }
        },
        None => { log::info!("auto_login: no saved username in store"); return None; }
    };

    log::info!("auto_login: found saved credentials for '{}' at {} (db: {})", username, url, database);

    // On desktop: load password from keyring. On mobile: load from store.
    #[cfg(desktop)]
    let password = match load_credentials(&username) {
        Ok(Some(pw)) => {
            log::info!("auto_login: loaded password from keyring");
            pw
        }
        Ok(None) => {
            log::warn!("auto_login: no password in keyring for '{}'", username);
            return None;
        }
        Err(e) => {
            log::error!("auto_login: keyring error: {e}");
            return None;
        }
    };

    #[cfg(mobile)]
    let password = match store.get(KEY_PASSWORD) {
        Some(v) => match v.as_str() {
            Some(s) => {
                log::info!("auto_login: loaded password from store");
                s.to_string()
            }
            None => { log::warn!("auto_login: password is not a string"); return None; }
        },
        None => { log::info!("auto_login: no saved password in store"); return None; }
    };

    log::info!("auto_login: connecting to {} as '{}'", url, username);

    match OdooClient::connect(&url, &database, &username, &password).await {
        Ok(client) => {
            let state = app_handle.state::<AppState>();
            let mut odoo_guard = state.odoo.lock().unwrap();
            *odoo_guard = Some(client);
            log::info!("auto_login: success!");
            Some(())
        }
        Err(e) => {
            log::error!("auto_login: connection failed: {e}");
            None
        }
    }
}
