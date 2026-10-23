use satchel_api::{ApiError, ErrorCode};

pub fn err(code: ErrorCode, msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(ApiError::new(code, msg))
}

pub fn not_found(what: &str) -> anyhow::Error {
    err(ErrorCode::NotFound, format!("{what} not found"))
}

pub fn invalid(msg: impl Into<String>) -> anyhow::Error {
    err(ErrorCode::InvalidInput, msg)
}

pub fn forbidden(msg: impl Into<String>) -> anyhow::Error {
    err(ErrorCode::Forbidden, msg)
}

/// Convert any error to the API error shape, preserving codes set with [`err`].
pub fn to_api(e: anyhow::Error) -> ApiError {
    if let Some(a) = e.downcast_ref::<ApiError>() {
        return a.clone();
    }
    let msg = format!("{e:#}");
    let lower = msg.to_lowercase();
    let code = if lower.contains("authentication") || lower.contains("auth failed") || lower.contains("invalid credentials") || lower.contains("login") && lower.contains("fail") {
        ErrorCode::Auth
    } else if lower.contains("connection") || lower.contains("timed out") || lower.contains("dns") || lower.contains("network") {
        ErrorCode::Network
    } else {
        ErrorCode::Internal
    };
    ApiError::new(code, msg)
}

/// Whether a provider error is worth retrying.
pub fn is_transient(e: &anyhow::Error) -> bool {
    if let Some(a) = e.downcast_ref::<ApiError>() {
        return matches!(a.code, ErrorCode::Network | ErrorCode::RateLimited | ErrorCode::Internal);
    }
    let s = format!("{e:#}").to_lowercase();
    !(s.contains("not found") || s.contains("invalid") || s.contains("forbidden") || s.contains("no such") || s.contains("permission"))
}
