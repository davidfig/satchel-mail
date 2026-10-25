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
    if e.downcast_ref::<crate::provider::MaybeSent>().is_some() {
        return false;
    }
    if let Some(a) = e.downcast_ref::<ApiError>() {
        return matches!(a.code, ErrorCode::Network | ErrorCode::RateLimited | ErrorCode::Internal);
    }
    // Network trouble is worth retrying; a refusal from the server isn't
    // (IMAP NO/BAD replies, a folder that's gone). Retrying a doomed op holds
    // back every op queued after it.
    if e.chain().any(|c| c.is::<std::io::Error>()) {
        return true;
    }
    const PERMANENT: [&str; 11] = [
        "not found",
        "invalid",
        "forbidden",
        "no such",
        "permission",
        "no response",
        "bad response",
        "already exists",
        "doesn't exist",
        "does not exist",
        "mailbox remote id",
    ];
    let s = format!("{e:#}").to_lowercase();
    !PERMANENT.iter().any(|p| s.contains(p))
}
