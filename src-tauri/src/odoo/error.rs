use crate::error::AppError;

/// Odoo-specific error that converts into the top-level AppError.
#[derive(Debug)]
pub struct OdooError(pub String);

impl std::fmt::Display for OdooError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OdooError {}

impl From<OdooError> for AppError {
    fn from(e: OdooError) -> Self {
        AppError::Odoo(e.0)
    }
}
