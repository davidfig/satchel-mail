//! The command/event schema every Satchel Mail host speaks.
//!
//! The UI (over Tauri IPC or HTTP+WebSocket) and the MCP server both talk to the
//! core exclusively through [`Command`] and [`CoreEvent`]. If it isn't expressible
//! here, no client can do it — which keeps the desktop app, the CLI and the
//! future hosted service at feature parity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type Id = String;

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub inline: bool,
    pub content_id: Option<String>,
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
    pub attachments: Vec<Attachment>,
    /// Body content is still being fetched.
    pub body_pending: bool,
    /// Calendar invitation carried by this message (iMIP), if any.
    #[serde(default)]
    pub invite: Option<Invite>,
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
    /// Search query (supports from:, to:, subject:, has:attachment, is:unread,
    /// is:flagged, before:, after:, in:).
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

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calendar {
    pub id: Id,
    pub account_id: Id,
    pub name: String,
    pub color: String,
    pub read_only: bool,
    /// Whether events in this calendar are shown.
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartStat {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
    pub status: PartStat,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Local id; None when creating.
    pub id: Option<Id>,
    pub calendar_id: Id,
    /// iCalendar UID.
    #[serde(default)]
    pub uid: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub all_day: bool,
    /// IANA zone the event was authored in.
    #[serde(default)]
    pub timezone: Option<String>,
    /// RFC 5545 RRULE value, e.g. "FREQ=WEEKLY;BYDAY=MO,WE".
    #[serde(default)]
    pub rrule: Option<String>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default)]
    pub organizer: Option<Address>,
    #[serde(default = "default_status")]
    pub status: EventStatus,
    /// The account owner's own participation status, if invited.
    #[serde(default)]
    pub my_status: Option<PartStat>,
}

fn default_status() -> EventStatus {
    EventStatus::Confirmed
}

/// One concrete occurrence of an event (recurrences expanded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Occurrence {
    pub event: Event,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeSlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Proposals (AI-originated changes awaiting human review)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProposalAction {
    /// A reply or new message. Never sent automatically; approving opens it in
    /// the composer.
    Draft { draft: Draft },
    Labels { message_ids: Vec<Id>, add: Vec<Id>, remove: Vec<Id> },
    Archive { message_ids: Vec<Id> },
    Move { message_ids: Vec<Id>, mailbox_id: Id },
    MarkRead { message_ids: Vec<Id>, seen: bool },
    Flag { message_ids: Vec<Id>, flagged: bool },
    CreateEvent { event: Event },
    UpdateEvent { event: Event },
    CancelEvent { event_id: Id },
    RespondToInvite { event_id: Id, response: PartStat },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Applied,
    Rejected,
    /// Target changed since the proposal was made; needs re-review.
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Id,
    pub agent_id: Id,
    pub agent_name: String,
    pub created_at: DateTime<Utc>,
    pub status: ProposalStatus,
    pub action: ProposalAction,
    pub rationale: String,
    /// Threads/events the agent based this on.
    pub source_thread_ids: Vec<Id>,
    /// Human-readable risk flags computed by the core (e.g. recipients outside
    /// the source thread).
    pub warnings: Vec<String>,
    pub review_note: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    /// One-line description for lists.
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Agents, settings, audit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPolicy {
    /// None = all accounts.
    pub account_ids: Option<Vec<Id>>,
    /// Only expose mail newer than this many days (None = everything).
    pub since_days: Option<u32>,
    pub allow_attachments: bool,
    pub allow_calendar: bool,
    pub max_proposals_per_hour: u32,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            account_ids: None,
            since_days: Some(365),
            allow_attachments: false,
            allow_calendar: true,
            max_proposals_per_hour: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Id,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub policy: AgentPolicy,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorSettings {
    pub enabled: bool,
    /// Defaults to ~/Satchel
    pub root: Option<String>,
    pub account_ids: Option<Vec<Id>>,
    pub since_days: Option<u32>,
    pub include_eml: bool,
    pub include_attachments: bool,
}

impl Default for MirrorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            root: None,
            account_ids: None,
            since_days: Some(365),
            include_eml: false,
            include_attachments: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub mirror: MirrorSettings,
    /// Senders whose remote images load automatically.
    pub remote_image_senders: Vec<String>,
    /// Push AI drafts to the server Drafts folder before approval (still never sent).
    pub sync_ai_drafts_to_server: bool,
    pub theme: String,
    /// Days of mail fetched on first sync before background backfill.
    pub initial_sync_days: u32,
    /// Seconds between polls for providers without push.
    pub poll_interval_secs: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mirror: MirrorSettings::default(),
            remote_image_senders: vec![],
            sync_ai_drafts_to_server: false,
            theme: "system".into(),
            initial_sync_days: 90,
            poll_interval_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    /// "user", "agent:<name>", "sync"
    pub actor: String,
    pub action: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: Id,
    pub account_id: Id,
    pub kind: String,
    pub status: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Every request a client can make. Serialized as `{"cmd": "...", "args": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Command {
    // accounts
    ListAccounts,
    TestAccount { config: AccountConfig, credential: Credential },
    AddAccount { config: AccountConfig, credential: Credential },
    RemoveAccount { account_id: Id },
    /// Change server settings (and optionally the credential) of an account.
    UpdateAccount { account_id: Id, config: AccountConfig, credential: Option<Credential> },
    /// Begin an OAuth flow; returns {auth_url}. The host completes it.
    StartOAuth { provider: ProviderKind },
    SyncNow { account_id: Option<Id> },

    // mail reads
    ListMailboxes { account_id: Option<Id> },
    ListThreads { query: ThreadQuery },
    /// Total matching threads (for list sizing); ignores paging fields.
    CountThreads { query: ThreadQuery },
    GetThread { thread_id: Id },
    GetMessage { message_id: Id },
    /// Raw RFC 822 source.
    GetRawMessage { message_id: Id },
    /// Returns {filename, mime_type, data_base64} or for agents extracted text.
    GetAttachment { message_id: Id, attachment_id: String },
    LoadRemoteImages { message_id: Id, always_for_sender: bool },

    // mail writes (user scope only)
    SetFlags { message_ids: Vec<Id>, seen: Option<bool>, flagged: Option<bool> },
    Move { message_ids: Vec<Id>, mailbox_id: Id },
    Archive { message_ids: Vec<Id> },
    Trash { message_ids: Vec<Id> },
    SetLabels { message_ids: Vec<Id>, add: Vec<Id>, remove: Vec<Id> },
    SaveDraft { draft: Draft },
    DeleteDraft { draft_id: Id },
    ListDrafts,
    Send { draft: Draft },
    /// Builds a reply/forward template for the composer.
    ComposeReply { message_id: Id, mode: ReplyMode },
    /// Address autocomplete from known correspondents.
    SuggestAddresses { prefix: String },

    // calendar
    ListCalendars,
    SetCalendarVisible { calendar_id: Id, visible: bool },
    ListEvents { from: DateTime<Utc>, to: DateTime<Utc>, calendar_ids: Option<Vec<Id>> },
    GetEvent { event_id: Id },
    CreateEvent { event: Event },
    UpdateEvent { event: Event },
    DeleteEvent { event_id: Id },
    RespondToInvite { event_id: Id, response: PartStat },
    /// Respond to an invitation received by email (adds it to a calendar or emails a reply).
    RespondToEmailInvite { message_id: Id, response: PartStat },
    FindFreeTime {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        duration_minutes: u32,
        /// Working hours in local time, e.g. 9..17.
        day_start_hour: Option<u32>,
        day_end_hour: Option<u32>,
        timezone: Option<String>,
    },

    // proposals
    ListProposals { status: Option<ProposalStatus> },
    GetProposal { proposal_id: Id },
    /// Agent scope: submit a proposal.
    CreateProposal { action: ProposalAction, rationale: String, source_thread_ids: Vec<Id> },
    /// User scope. `edited` replaces the action before applying.
    ApproveProposal { proposal_id: Id, edited: Option<ProposalAction> },
    RejectProposal { proposal_id: Id, note: Option<String> },

    // agents
    ListAgents,
    /// Returns {agent, token}. The token is shown once.
    CreateAgent { name: String, policy: AgentPolicy },
    UpdateAgent { agent_id: Id, name: Option<String>, policy: Option<AgentPolicy> },
    RevokeAgent { agent_id: Id },

    // misc
    GetSettings,
    UpdateSettings { settings: Settings },
    ListAudit { limit: Option<u32> },
    ListOutbox,
    RetryOutbox { id: Id },
    /// Returns basic info about the running core.
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    Reply,
    ReplyAll,
    Forward,
}

impl Command {
    /// Commands an AI agent may issue. Everything else requires the user scope.
    pub fn allowed_for_agent(&self) -> bool {
        matches!(
            self,
            Command::ListAccounts
                | Command::ListMailboxes { .. }
                | Command::ListThreads { .. }
                | Command::CountThreads { .. }
                | Command::GetThread { .. }
                | Command::GetMessage { .. }
                | Command::GetAttachment { .. }
                | Command::ListCalendars
                | Command::ListEvents { .. }
                | Command::GetEvent { .. }
                | Command::FindFreeTime { .. }
                | Command::CreateProposal { .. }
                | Command::GetProposal { .. }
                | Command::ListProposals { .. }
                | Command::Info
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    Forbidden,
    Auth,
    Network,
    Conflict,
    RateLimited,
    Internal,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Wire envelope for request/response transports (HTTP, WebSocket, stdio).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

// ---------------------------------------------------------------------------
// Events (core -> clients)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CoreEvent {
    AccountsChanged,
    SyncStatus { account_id: Id, status: SyncStatus },
    MailboxesChanged { account_id: Id },
    MessagesChanged { account_id: Id },
    CalendarChanged { account_id: Id },
    ProposalsChanged,
    OutboxChanged,
    /// A queued operation failed permanently.
    OpFailed { account_id: Id, description: String, error: String },
    /// Another process changed the database.
    DataChanged,
    Notice { level: String, message: String },
}

