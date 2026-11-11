// SPDX-License-Identifier: MIT OR Apache-2.0
//! Settings sync between computers through a shared folder (0080).

use super::*;

/// What `GetSync` returns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderSync {
    /// A sync folder is set on this computer.
    pub enabled: bool,
    pub folder: Option<String>,
    /// This computer's id and name, as other computers see it.
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    /// The other computers whose files are in the folder.
    pub devices: Vec<SyncDevice>,
    /// When the last pass finished.
    pub last_pass: Option<DateTime<Utc>>,
    /// Why the last pass failed (wrong code, folder missing, …).
    pub error: Option<String>,
    /// Accounts set up on another computer and not here yet.
    pub offered_accounts: Vec<OfferedAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDevice {
    pub id: String,
    pub name: String,
    /// When that computer last wrote its file.
    pub last_seen: DateTime<Utc>,
}

/// An account from another computer. Signing in adds it here with the same
/// server setup; its password or OAuth token never travels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferedAccount {
    pub config: AccountConfig,
    pub color: String,
}
