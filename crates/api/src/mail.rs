//! Mail: mailboxes, messages, threads, drafts, warnings and sender rules.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    All,
    Flagged,
    Important,
    /// A user folder or label.
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: Id,
    pub account_id: Id,
    pub name: String,
    /// Full provider path, e.g. "Projects/2026".
    pub path: String,
    pub role: MailboxRole,
    /// True for Gmail-style labels (a message can be in many).
    pub is_label: bool,
    pub unread: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(email: impl Into<String>) -> Self {
        Self { name: None, email: email.into() }
    }
    pub fn display(&self) -> String {
        match &self.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", self.email),
            _ => self.email.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: Id,
    pub account_id: Id,
    pub thread_id: Id,
    pub subject: String,
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub date: DateTime<Utc>,
    pub snippet: String,
    pub flags: Flags,
    pub mailbox_ids: Vec<Id>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub thread_id: Id,
    pub account_id: Id,
    pub subject: String,
    pub participants: Vec<Address>,
    pub last_date: DateTime<Utc>,
    pub message_count: u32,
    pub unread_count: u32,
    pub flagged: bool,
    pub has_attachments: bool,
    pub snippet: String,
    pub mailbox_ids: Vec<Id>,
    pub latest_message_id: Id,
}

/// Everything exchanged with one person, across all of their addresses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonSummary {
    /// The contact these addresses belong to, if any. Its other addresses are
    /// included in `addresses`.
    pub contact_id: Option<Id>,
    /// The contact's name, else the name they use most often.
    pub name: Option<String>,
    pub addresses: Vec<Address>,
    /// Messages from them.
    pub received: u32,
    /// Messages you sent them (To, Cc or Bcc).
    pub sent: u32,
    /// Conversations they are in (same as a `with:` search).
    pub threads: u32,
    /// Messages from them with attachments.
    pub attachments: u32,
    pub first_date: Option<DateTime<Utc>>,
    pub last_date: Option<DateTime<Utc>>,
    /// Events they organize or attend, newest first (empty when calendar access is off).
    pub events: Vec<PersonEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonEvent {
    pub event_id: Id,
    pub title: String,
    /// For a recurring event, when the series starts.
    pub start: DateTime<Utc>,
    pub recurring: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub inline: bool,
    pub content_id: Option<String>,
    /// Executable or script-like content (HTML pages, programs, macro documents).
    #[serde(default)]
    pub risky: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDetail {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub text_body: String,
    /// Sanitized HTML, safe to render in a sandboxed iframe. Remote images are
    /// rewritten to placeholders unless allowed for this sender.
    pub html_body: Option<String>,
    pub remote_images_blocked: bool,
    /// Whether the message may load remote content (allowed for its sender,
    /// or this once). When false the client must block it too, not rely on
    /// the sanitizer having found every remote reference.
    #[serde(default)]
    pub remote_images_allowed: bool,
    pub attachments: Vec<Attachment>,
    /// Body content is still being fetched.
    pub body_pending: bool,
    /// Calendar invitation carried by this message (iMIP), if any.
    #[serde(default)]
    pub invite: Option<Invite>,
    /// Signs this message may be phishing or unsafe, computed by the core.
    #[serde(default)]
    pub warnings: Vec<MessageWarning>,
    /// How to unsubscribe, from the List-Unsubscribe header.
    #[serde(default)]
    pub unsubscribe: Option<Unsubscribe>,
    /// The user's standing verdict on the sender, if any.
    #[serde(default)]
    pub sender_verdict: Option<SenderVerdict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// SPF/DKIM/DMARC checks by the receiving server failed.
    AuthFailed,
    /// The display name matches someone you correspond with, but the address doesn't.
    Impersonation,
    /// Replies would go to a different domain than the sender's.
    ReplyTo,
    /// A link's text shows one domain but it goes to another.
    DeceptiveLink,
    RiskyAttachment,
    /// A calendar invitation from someone you've never written to.
    UnknownInviter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWarning {
    pub kind: WarningKind,
    pub message: String,
}

/// Unsubscribe methods a message offers (RFC 2369 / RFC 8058).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsubscribe {
    /// https URL to open, or to POST to when `one_click`.
    pub url: Option<String>,
    pub mailto: Option<String>,
    /// The sender supports RFC 8058 one-click unsubscribe at `url`.
    pub one_click: bool,
}

/// A standing decision about every message from one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderVerdict {
    /// Always goes to Spam.
    Spam,
    /// Never goes to Spam.
    NotSpam,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderRule {
    pub email: String,
    pub verdict: SenderVerdict,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub uid: String,
    /// REQUEST, CANCEL, REPLY...
    pub method: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub location: String,
    pub organizer: Option<Address>,
    pub my_status: Option<PartStat>,
    /// The matching event in one of the user's calendars, if synced.
    pub event_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub thread_id: Id,
    pub account_id: Id,
    pub subject: String,
    pub messages: Vec<MessageDetail>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadQuery {
    /// Specific mailbox.
    pub mailbox_id: Option<Id>,
    /// Unified view across accounts by role (e.g. all inboxes).
    pub role: Option<MailboxRole>,
    pub account_id: Option<Id>,
    /// Search query (supports from:, to:, with:, subject:, has:attachment,
    /// is:unread, is:flagged, before:, after:, in:). from:, to: and with: take a
    /// comma-separated list; a full address matches exactly.
    pub search: Option<String>,
    pub limit: Option<u32>,
    /// Keyset paging: threads older than this `last_date`...
    pub before: Option<DateTime<Utc>>,
    /// ...or at exactly that date with a smaller thread id (ties are common).
    #[serde(default)]
    pub before_id: Option<Id>,
    /// Offset paging (random access for windowed lists).
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Draft {
    /// Local draft id; None for a new draft.
    pub id: Option<Id>,
    pub account_id: Id,
    pub to: Vec<Address>,
    #[serde(default)]
    pub cc: Vec<Address>,
    #[serde(default)]
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    /// Local message id this replies to (sets In-Reply-To/References).
    #[serde(default)]
    pub in_reply_to: Option<Id>,
    /// Local message id this forwards (attachments are carried over).
    #[serde(default)]
    pub forward_of: Option<Id>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    /// Base64 content.
    pub data_base64: String,
}
