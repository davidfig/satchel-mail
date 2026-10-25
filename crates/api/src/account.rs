//! Accounts: providers, connection settings, credentials and sync status.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Imap,
    Gmail,
    Graph,
    Jmap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    Tls,
    StartTls,
    None,
}

/// Everything needed to (re)connect an account. Secrets are never part of
/// this struct once stored; they live in the secret store keyed by account id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountConfig {
    Imap {
        email: String,
        display_name: String,
        username: String,
        imap_host: String,
        imap_port: u16,
        imap_security: Security,
        smtp_host: String,
        smtp_port: u16,
        smtp_security: Security,
        /// Optional CalDAV collection-home or principal URL.
        #[serde(default)]
        caldav_url: Option<String>,
        /// Optional CardDAV URL; when empty, contacts are looked up on the CalDAV server.
        #[serde(default)]
        carddav_url: Option<String>,
        /// Accept self-signed certificates (local test servers only).
        #[serde(default)]
        allow_invalid_certs: bool,
    },
    Jmap {
        email: String,
        display_name: String,
        username: String,
        /// e.g. https://api.fastmail.com/jmap/session
        session_url: String,
        #[serde(default)]
        caldav_url: Option<String>,
        #[serde(default)]
        carddav_url: Option<String>,
        #[serde(default)]
        allow_invalid_certs: bool,
    },
    Gmail {
        email: String,
        display_name: String,
    },
    Graph {
        email: String,
        display_name: String,
    },
}

impl AccountConfig {
    pub fn kind(&self) -> ProviderKind {
        match self {
            AccountConfig::Imap { .. } => ProviderKind::Imap,
            AccountConfig::Jmap { .. } => ProviderKind::Jmap,
            AccountConfig::Gmail { .. } => ProviderKind::Gmail,
            AccountConfig::Graph { .. } => ProviderKind::Graph,
        }
    }
    pub fn email(&self) -> &str {
        match self {
            AccountConfig::Imap { email, .. }
            | AccountConfig::Jmap { email, .. }
            | AccountConfig::Gmail { email, .. }
            | AccountConfig::Graph { email, .. } => email,
        }
    }
    pub fn display_name(&self) -> &str {
        match self {
            AccountConfig::Imap { display_name, .. }
            | AccountConfig::Jmap { display_name, .. }
            | AccountConfig::Gmail { display_name, .. }
            | AccountConfig::Graph { display_name, .. } => display_name,
        }
    }
}

/// Credentials supplied when adding an account. Stored in the secret store,
/// never in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    Password { password: String },
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        /// Unix seconds.
        expires_at: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Id,
    pub kind: ProviderKind,
    pub email: String,
    pub display_name: String,
    pub config: AccountConfig,
    pub sync: SyncStatus,
    pub color: String,
    /// Signed in before Satchel asked for access to contacts: contacts don't
    /// sync until the user signs in again.
    #[serde(default)]
    pub contacts_need_reconnect: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStatus {
    pub state: SyncState,
    pub last_success: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Free-form progress text ("Fetching INBOX 120/900").
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    #[default]
    Idle,
    Syncing,
    Error,
    Offline,
}
