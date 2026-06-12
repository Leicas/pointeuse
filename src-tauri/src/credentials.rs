use crate::error::AppResult;
#[cfg(desktop)]
use crate::error::AppError;

const SERVICE: &str = "time-tracker-app";

/// Store an Odoo password/API-key in the OS credential store.
#[cfg(desktop)]
pub fn save_credentials(username: &str, password: &str) -> AppResult<()> {
    log::info!("credentials: saving for '{username}'");
    let entry = keyring::Entry::new(SERVICE, username)
        .map_err(|e| {
            log::error!("credentials: failed to create entry: {e}");
            AppError::Keyring(e.to_string())
        })?;
    entry
        .set_password(password)
        .map_err(|e| {
            log::error!("credentials: failed to set password: {e}");
            AppError::Keyring(e.to_string())
        })?;
    log::info!("credentials: saved successfully");
    Ok(())
}

/// Load the stored password for `username`. Returns `Ok(None)` when no
/// credential exists (rather than treating it as an error).
#[cfg(desktop)]
pub fn load_credentials(username: &str) -> AppResult<Option<String>> {
    log::info!("credentials: loading for '{username}'");
    let entry = keyring::Entry::new(SERVICE, username)
        .map_err(|e| {
            log::error!("credentials: failed to create entry for load: {e}");
            AppError::Keyring(e.to_string())
        })?;
    match entry.get_password() {
        Ok(pw) => {
            log::info!("credentials: loaded successfully");
            Ok(Some(pw))
        }
        Err(keyring::Error::NoEntry) => {
            log::warn!("credentials: no entry found for '{username}'");
            Ok(None)
        }
        Err(e) => {
            log::error!("credentials: failed to get password: {e}");
            Err(AppError::Keyring(e.to_string()))
        }
    }
}

/// Remove the stored credential for `username`.
#[cfg(desktop)]
pub fn clear_credentials(username: &str) -> AppResult<()> {
    let entry = keyring::Entry::new(SERVICE, username)
        .map_err(|e| AppError::Keyring(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Keyring(e.to_string())),
    }
}

// On mobile, credentials are stored in the Tauri plugin-store (no keyring available).

#[cfg(mobile)]
pub fn save_credentials(_username: &str, _password: &str) -> AppResult<()> {
    // On mobile, password is stored via the Tauri store in auth.rs
    Ok(())
}

#[cfg(mobile)]
pub fn load_credentials(_username: &str) -> AppResult<Option<String>> {
    // On mobile, password is loaded from the Tauri store in auth.rs
    Ok(None)
}

#[cfg(mobile)]
pub fn clear_credentials(_username: &str) -> AppResult<()> {
    Ok(())
}
