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

/// The account status for a failed sync pass. Only a provider's own auth error
/// means the user must sign in again: words in an untyped error's text can't
/// tell the user that their account was signed out.
pub(crate) fn sync_state_for(e: &anyhow::Error) -> satchel_api::SyncState {
    use satchel_api::SyncState;
    let typed = e.chain().find_map(|c| c.downcast_ref::<ApiError>()).map(|a| a.code);
    match typed {
        Some(ErrorCode::Auth) => SyncState::SignIn,
        Some(ErrorCode::Network) => SyncState::Offline,
        Some(_) => SyncState::Error,
        None => match to_api(anyhow::anyhow!("{e:#}")).code {
            ErrorCode::Network => SyncState::Offline,
            _ => SyncState::Error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, anyhow};
    use satchel_api::SyncState;

    #[test]
    fn only_a_typed_auth_error_means_signed_out() {
        let auth = || anyhow::Error::new(ApiError::new(ErrorCode::Auth, "access was revoked; sign in again"));
        assert_eq!(sync_state_for(&auth()), SyncState::SignIn);
        assert_eq!(sync_state_for(&auth().context("sync mail")), SyncState::SignIn);
        assert_eq!(sync_state_for(&anyhow!("TLS authentication tag mismatch")), SyncState::Error);
        assert_eq!(sync_state_for(&anyhow!("parsing message: Login failed notice")), SyncState::Error);
    }

    #[test]
    fn other_failures_keep_their_state() {
        assert_eq!(sync_state_for(&anyhow::Error::new(ApiError::new(ErrorCode::Network, "HTTP 503"))), SyncState::Offline);
        assert_eq!(sync_state_for(&anyhow!("connection reset")), SyncState::Offline);
        assert_eq!(sync_state_for(&anyhow::Error::new(ApiError::new(ErrorCode::RateLimited, "slow down"))), SyncState::Error);
        assert_eq!(sync_state_for(&Err::<(), _>(anyhow!("boom")).context("sync").unwrap_err()), SyncState::Error);
    }
}
