//! Records the store reads and writes, beyond the API types.

use super::*;

/// A message as written by sync.
#[derive(Debug, Clone, Default)]
pub struct MessageRecord {
    pub remote_id: String,
    pub remote_thread_id: Option<String>,
    pub message_id_hdr: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: DateTime<Utc>,
    pub snippet: String,
    pub flags: Flags,
    pub attachments: Vec<Attachment>,
    pub blob_hash: Option<String>,
    pub size: u64,
    /// With `blob_hash`: the copy leaves out attachment contents.
    pub partial: bool,
    /// With `blob_hash`: bytes the copy takes on disk.
    pub blob_size: u64,
}

/// Remote handles for a local message, used when pushing ops.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct MessageRef {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub remote_thread_id: Option<String>,
    pub message_id_hdr: Option<String>,
    /// (mailbox local id, mailbox remote id, uid)
    pub mailboxes: Vec<(String, String, Option<u32>)>,
    pub flags: Flags,
    pub blob_hash: Option<String>,
    /// The full message's size on the server, if known (0 if not).
    pub size: u64,
    pub version: i64,
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: String,
    pub calendar_id: String,
    pub account_id: String,
    pub remote_id: String,
    pub etag: Option<String>,
    pub uid: String,
    pub ics: String,
    pub version: i64,
    pub pending: bool,
}

#[derive(Debug, Clone)]
pub struct StoredCalendar {
    pub cal: Calendar,
    pub remote_id: String,
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredContact {
    pub id: String,
    pub fields: ContactFields,
    pub origin: ContactOrigin,
    pub deleted: bool,
    pub version: i64,
}

#[derive(Debug, Clone)]
pub struct StoredAddressBook {
    pub book: AddressBook,
    pub remote_id: String,
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredCard {
    pub id: String,
    pub contact_id: String,
    pub address_book_id: String,
    pub account_id: String,
    pub remote_id: String,
    pub etag: Option<String>,
    pub uid: String,
    /// Last version the server confirmed.
    pub vcard: String,
}

/// One sign of correspondence: (address, display name, date ms, whether the
/// name is the one they gave themselves).
pub type Evidence = (String, Option<String>, i64, bool);

/// Someone the user corresponds with (derived from mail).
#[derive(Debug, Clone, PartialEq)]
pub struct Correspondent {
    /// Lowercase.
    pub email: String,
    pub name: Option<String>,
    pub messages: u32,
    pub last_date: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OpRow {
    pub id: String,
    pub account_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub attempts: u32,
    pub actor: String,
}


#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub summary: MessageSummary,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub references: Vec<String>,
    pub message_id_hdr: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachments: Vec<Attachment>,
    pub blob_hash: Option<String>,
    /// The stored copy leaves out attachment contents.
    pub partial: bool,
    pub remote_id: String,
    pub version: i64,
}

/// One message's searchable text.
#[derive(Debug, Clone, Default)]
pub struct SearchDoc {
    pub id: String,
    pub subject: String,
    pub participants: String,
    pub body: String,
    pub attachments: String,
    /// The body version this entry was built from.
    pub blob_hash: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ThreadFilter {
    pub mailbox_id: Option<String>,
    pub role: Option<MailboxRole>,
    pub account_ids: Option<Vec<String>>,
    pub exclude_trash_junk: bool,
    /// Conditions that must all hold for one message of the thread (from a search).
    pub conds: Vec<MsgCond>,
    /// Only messages from this date on (an agent's date range).
    pub after: Option<DateTime<Utc>>,
    pub message_ids: Option<Vec<String>>,
    pub before: Option<DateTime<Utc>>,
    /// Tie-breaker for `before`: threads at exactly `before` with a smaller id.
    pub before_id: Option<String>,
    pub offset: u32,
    pub limit: u32,
}

/// One search condition on a message (see `mail::search`).
#[derive(Debug, Clone, PartialEq)]
pub enum MsgTerm {
    /// Each address list matches any of its values; a full address matches exactly.
    From(Vec<String>),
    To(Vec<String>),
    /// Sender or recipient.
    With(Vec<String>),
    Subject(String),
    HasAttachment,
    Unread,
    Flagged,
    Before(DateTime<Utc>),
    After(DateTime<Utc>),
    /// In a mailbox with one of these names or roles.
    InMailbox(Vec<String>),
    /// FTS5 MATCH expression (see `search::fts_query`).
    Text(String),
}

/// A [`MsgTerm`] that must hold, or with `negate`, must not.
#[derive(Debug, Clone, PartialEq)]
pub struct MsgCond {
    pub term: MsgTerm,
    pub negate: bool,
}

impl MsgCond {
    pub fn new(term: MsgTerm) -> Self {
        Self { term, negate: false }
    }
}
