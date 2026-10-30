//! Commands: everything a client can ask the core to do.

use super::*;

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
    // folders (user scope only)
    /// New folder (or label) in an account, at the top level or inside
    /// `parent_id` (one of its own folders). Returns the Mailbox.
    CreateMailbox { account_id: Id, name: String, parent_id: Option<Id> },
    /// Rename a folder and/or move it under another of the account's folders
    /// (`parent_id: null` = top level). Its subfolders come along.
    UpdateMailbox { mailbox_id: Id, name: String, parent_id: Option<Id> },
    /// Delete an empty-of-subfolders folder. Its messages move to Trash first
    /// (labels: the label just goes). Returns {moved_to_trash}.
    DeleteMailbox { mailbox_id: Id },
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
    /// Mark every unread message in one mailbox, or in every mailbox with a
    /// role, as read. Returns {message_ids} (what changed, for undo).
    MarkAllRead { mailbox_id: Option<Id>, role: Option<MailboxRole> },
    /// Permanently delete everything in a Spam or Trash folder, or in every
    /// one with that role. Returns {deleted}.
    EmptyMailbox { mailbox_id: Option<Id>, role: Option<MailboxRole> },
    Move { message_ids: Vec<Id>, mailbox_id: Id },
    Archive { message_ids: Vec<Id> },
    Trash { message_ids: Vec<Id> },
    SetLabels { message_ids: Vec<Id>, add: Vec<Id>, remove: Vec<Id> },
    /// Move to Spam and send all future mail from these senders there.
    MarkSpam { message_ids: Vec<Id> },
    /// Move back to the inbox and never send these senders to Spam again.
    MarkNotSpam { message_ids: Vec<Id> },
    ListSenderRules,
    /// Set or (with `verdict: null`) remove the rule for one address.
    SetSenderRule { email: String, verdict: Option<SenderVerdict> },
    /// Favorite sender addresses (lowercase), sorted (0053).
    ListFavoriteSenders,
    /// Add addresses to the favorites, or remove them: all of one person's
    /// addresses at once. Their conversations are highlighted and match
    /// `is:favorite`.
    SetFavoriteSenders { emails: Vec<String>, favorite: bool },
    /// Unsubscribe using the message's List-Unsubscribe header. Returns
    /// {via: "one_click" | "email" | "browser", url?}; "browser" means the
    /// client should open `url`.
    Unsubscribe { message_id: Id },
    SaveDraft { draft: Draft },
    DeleteDraft { draft_id: Id },
    ListDrafts,
    /// A message in Drafts as an editable draft: Satchel's own draft, or (for
    /// one written in another client) a new local draft that replaces the
    /// server copy when saved or sent.
    EditDraft { message_id: Id },
    Send { draft: Draft },
    /// Builds a reply/forward template for the composer.
    ComposeReply { message_id: Id, mode: ReplyMode },
    /// Address autocomplete from known correspondents.
    SuggestAddresses { prefix: String },
    /// Summary of what you have exchanged with one person, given any of their
    /// addresses (a contact's other addresses are added). List their
    /// conversations with a `with:` search over the returned addresses.
    GetPerson { emails: Vec<String> },

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
    /// Places for an event's location: ones you've used, then (if the user
    /// turned on `look_up_places`) a lookup with the host's place service.
    SuggestLocations { query: String },

    // contacts
    /// Contacts sorted by name, optionally filtered by name, address or organization.
    ListContacts { search: Option<String>, limit: Option<u32> },
    GetContact { contact_id: Id },
    ListAddressBooks,
    /// Create (no id) or update a contact. `address_book_id` also saves it to
    /// that server address book (it is local-only otherwise).
    SaveContact { contact: Contact, address_book_id: Option<Id> },
    /// Delete a contact (from its server address books too). Automatic
    /// generation never brings it back.
    DeleteContact { contact_id: Id },
    /// Merge the other contacts into the first one.
    MergeContacts { contact_ids: Vec<Id> },

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
                | Command::ListContacts { .. }
                | Command::GetContact { .. }
                | Command::ListAddressBooks
                | Command::CreateProposal { .. }
                | Command::GetProposal { .. }
                | Command::ListProposals { .. }
                | Command::Info
        )
    }
}
