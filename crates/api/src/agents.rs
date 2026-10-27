//! Agents, their proposals (changes awaiting the user's review), settings and the audit log.

use super::*;

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
    /// Create a contact (no id) or change one. Never deletes or merges.
    SaveContact { contact: Contact },
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "AgentPolicyWire")]
pub struct AgentPolicy {
    /// None = all accounts.
    pub account_ids: Option<Vec<Id>>,
    /// Only expose mail newer than this many days (None = everything).
    pub since_days: Option<u32>,
    pub allow_attachments: bool,
    pub allow_calendar: bool,
    /// Read contacts (and propose changes to them). Policies saved before
    /// contacts existed get the same answer as `allow_calendar`.
    pub allow_contacts: bool,
    pub max_proposals_per_hour: u32,
}

/// AgentPolicy as stored, where `allow_contacts` may be missing.
#[derive(Deserialize)]
struct AgentPolicyWire {
    account_ids: Option<Vec<Id>>,
    since_days: Option<u32>,
    allow_attachments: bool,
    allow_calendar: bool,
    #[serde(default)]
    allow_contacts: Option<bool>,
    max_proposals_per_hour: u32,
}

impl From<AgentPolicyWire> for AgentPolicy {
    fn from(w: AgentPolicyWire) -> Self {
        Self {
            account_ids: w.account_ids,
            since_days: w.since_days,
            allow_attachments: w.allow_attachments,
            allow_calendar: w.allow_calendar,
            allow_contacts: w.allow_contacts.unwrap_or(w.allow_calendar),
            max_proposals_per_hour: w.max_proposals_per_hour,
        }
    }
}

impl Default for AgentPolicy {
    fn default() -> Self {
        let allow_calendar = true;
        Self {
            account_ids: None,
            since_days: Some(365),
            allow_attachments: false,
            allow_calendar,
            allow_contacts: allow_calendar,
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
    /// Built into Satchel (the AI folder), not created by the user.
    #[serde(default)]
    pub builtin: bool,
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
    /// Turn files in `proposals/inbox/` into proposals without an agent token.
    #[serde(default = "yes")]
    pub accept_proposals: bool,
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
            accept_proposals: true,
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
    /// Days of mail whose content downloads in the background (None: all
    /// mail). Older mail downloads when opened.
    #[serde(default)]
    pub download_days: Option<u32>,
    /// Most disk space each account's downloaded mail may use, in MB (None:
    /// no limit). Over it, the oldest content is freed for newer mail.
    #[serde(default)]
    pub storage_limit_mb: Option<u64>,
    /// Seconds between polls for providers without push.
    pub poll_interval_secs: u32,
    /// Show a desktop notification when new mail arrives in an inbox.
    #[serde(default = "yes")]
    pub notify_new_mail: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mirror: MirrorSettings::default(),
            remote_image_senders: vec![],
            sync_ai_drafts_to_server: false,
            theme: "system".into(),
            initial_sync_days: 90,
            download_days: None,
            storage_limit_mb: None,
            poll_interval_secs: 120,
            notify_new_mail: true,
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
