//! Events the core pushes to clients.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CoreEvent {
    AccountsChanged,
    SyncStatus { account_id: Id, status: SyncStatus },
    MailboxesChanged { account_id: Id },
    MessagesChanged { account_id: Id },
    CalendarChanged { account_id: Id },
    ContactsChanged,
    /// The favorite senders changed (0053).
    FavoritesChanged,
    ProposalsChanged,
    OutboxChanged,
    /// A queued operation failed permanently.
    OpFailed { account_id: Id, description: String, error: String },
    /// Another process changed the database.
    DataChanged,
    Notice { level: String, message: String },
    /// Unread mail just arrived in an inbox (not on first sync or backfill).
    NewMail { account_id: Id, messages: Vec<NewMail> },
    /// Calendar alerts changed (0056): `due` went off just now; `closed`
    /// stopped showing (dismissed, snoozed, or their event changed), so a
    /// host can take down their notifications.
    AlertsChanged { due: Vec<CalendarAlert>, closed: Vec<Id> },
}

/// One arrival, enough for a desktop notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMail {
    pub message_id: Id,
    pub thread_id: Id,
    pub from: Option<Address>,
    pub subject: String,
    pub snippet: String,
}
