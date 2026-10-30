//! The contract between the core and provider adapters.
//!
//! Providers do protocol work only. They write what they find through
//! [`SyncCtx`], which owns all persistence, threading, indexing and events.

pub mod priority;
mod sync_ctx;

pub use sync_ctx::SyncCtx;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use satchel_api::*;

use crate::blobs::BlobStore;
use crate::secrets::{SecretStore, account_key};
use crate::store::{MessageRecord, MessageRef, Store};

/// A mailbox as reported by the provider.
#[derive(Debug, Clone)]
pub struct RemoteMailbox {
    pub remote_id: String,
    pub name: String,
    pub path: String,
    pub role: MailboxRole,
    pub is_label: bool,
}

/// A send that failed after the message may already have reached the server
/// (the connection dropped before the server confirmed it). The core never
/// retries these on its own: a retry could deliver the message twice.
#[derive(Debug)]
pub struct MaybeSent(pub anyhow::Error);

impl std::fmt::Display for MaybeSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the server didn't confirm the send, so it may have gone out; check Sent before retrying ({:#})", self.0)
    }
}

impl std::error::Error for MaybeSent {}

impl MaybeSent {
    /// Classify an error from the final step of a send. Refusals the server
    /// answered (4xx) and connections that never opened mean nothing was sent
    /// and pass through unchanged; anything else may have been sent.
    pub fn classify(e: anyhow::Error) -> anyhow::Error {
        let refused = e.downcast_ref::<ApiError>().is_some_and(|a| {
            matches!(a.code, ErrorCode::Auth | ErrorCode::Forbidden | ErrorCode::NotFound | ErrorCode::InvalidInput | ErrorCode::RateLimited | ErrorCode::Conflict)
        });
        if refused || e.downcast_ref::<MaybeSent>().is_some() {
            return e;
        }
        anyhow::Error::new(MaybeSent(e))
    }
}

/// A mailbox membership as (remote mailbox id, IMAP UID if any).
pub type RemoteMembership = (String, Option<u32>);

/// What a provider did when applying an op, so the core can reconcile.
#[derive(Debug, Clone, Default)]
pub struct ApplyOutcome {
    /// New remote identity for moved messages: (local message id, new remote id, memberships as remote mailbox ids + uids).
    pub rekeyed: Vec<(String, String, Vec<RemoteMembership>)>,
    /// For SaveDraftRemote / Send: remote id of the stored copy.
    pub stored_remote_id: Option<String>,
    /// For calendar writes.
    pub event_remote_id: Option<String>,
    pub event_etag: Option<String>,
    /// For contact writes: where the card now lives, its etag, and the card as
    /// the server stores it if the provider knows (else what was sent is assumed).
    pub card_remote_id: Option<String>,
    pub card_etag: Option<String>,
    pub card_vcard: Option<String>,
    /// For mailbox creates: the new folder's remote id.
    pub mailbox_remote_id: Option<String>,
    /// For mailbox updates that change remote ids (IMAP names are paths): old → new,
    /// for the folder and each folder under it.
    pub mailbox_renamed: Vec<(String, String)>,
}

/// A mutation, with remote handles already resolved by the core.
#[derive(Debug, Clone)]
pub enum ResolvedOp {
    SetFlags { messages: Vec<MessageRef>, seen: Option<bool>, flagged: Option<bool>, answered: Option<bool> },
    /// Move to a mailbox (folder semantics: removes from source).
    Move { messages: Vec<MessageRef>, to_remote_mailbox: String },
    /// Label semantics: add/remove memberships without moving.
    Labels { messages: Vec<MessageRef>, add: Vec<String>, remove: Vec<String> },
    /// Permanently delete (only from Trash).
    Expunge { messages: Vec<MessageRef> },
    /// Store a draft copy on the server, replacing `replaces` if set.
    SaveDraft { raw: Vec<u8>, replaces: Option<MessageRef>, drafts_remote_mailbox: Option<String> },
    DeleteDraft { message: MessageRef },
    Send {
        raw: Vec<u8>,
        from: String,
        rcpts: Vec<String>,
        sent_remote_mailbox: Option<String>,
        replaces_draft: Option<MessageRef>,
        /// Provider thread of the message being replied to (Gmail needs it to thread).
        thread_remote_id: Option<String>,
    },
    /// New folder `name` under `parent` (None = top level). `path` is the full
    /// "/"-separated path, for providers that name folders by path (Gmail).
    CreateMailbox { name: String, parent: Option<String>, path: String },
    /// Rename and/or move a folder; folders under it come along.
    UpdateMailbox { remote_id: String, old_path: String, name: String, parent: Option<String>, path: String },
    /// Delete a folder. The core has already moved its messages out.
    DeleteMailbox { remote_id: String, path: String },
}

#[derive(Debug, Clone)]
pub struct RemoteCalendar {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub enum CalendarOp {
    Put { calendar_remote_id: String, event_remote_id: Option<String>, etag: Option<String>, uid: String, ics: String },
    /// `notify: false` removes an invitation silently: no decline goes to the organizer.
    Delete { calendar_remote_id: String, event_remote_id: String, etag: Option<String>, notify: bool },
}

#[derive(Debug, Clone)]
pub struct RemoteAddressBook {
    pub remote_id: String,
    pub name: String,
    pub read_only: bool,
}

/// Contact writes speak vCard, whatever the provider's own format, the way
/// calendar writes speak iCalendar.
#[derive(Debug, Clone)]
pub enum ContactOp {
    /// Create (no `card_remote_id`) or replace a card. `etag` guards against
    /// overwriting a server change: a mismatch is an `ErrorCode::Conflict`.
    Put { book_remote_id: String, card_remote_id: Option<String>, etag: Option<String>, uid: String, vcard: String },
    Delete { book_remote_id: String, card_remote_id: String, etag: Option<String> },
}

/// A message as downloaded for its stored copy.
#[derive(Debug, Clone)]
pub enum Download {
    /// The raw message as the server has it.
    Full(Vec<u8>),
    /// Rebuilt without attachment contents, which never left the server.
    Slim(satchel_mime::Slim),
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    /// Verify credentials and connectivity.
    async fn test(&self) -> Result<()>;
    /// Bring recent mail up to date (delta when possible): new messages,
    /// flags and deletions. Older mail is left to [`Self::backfill`] (0047).
    async fn sync(&self, ctx: &SyncCtx) -> Result<()>;
    /// Fetch older mail, newest first, until `until` or none is left, and
    /// call [`SyncCtx::more_to_sync`] if some is. Runs after [`Self::sync`]
    /// in the same pass. Default: nothing to fetch.
    async fn backfill(&self, _ctx: &SyncCtx, _until: std::time::Instant) -> Result<()> {
        Ok(())
    }
    /// Fetch a full raw message.
    async fn fetch_raw(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>>;
    /// Fetch a message for the stored copy, leaving attachment contents on
    /// the server where it can (0036). Default: the full message, which
    /// [`SyncCtx::put_blob`] strips after downloading.
    async fn fetch_without_attachments(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Download> {
        Ok(Download::Full(self.fetch_raw(ctx, msg).await?))
    }
    async fn apply(&self, ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome>;
    /// Block until the server signals changes or `max_wait` elapses. Returns
    /// true if changes are likely. Default: plain sleep (polling).
    async fn wait_for_changes(&self, _ctx: &SyncCtx, max_wait: std::time::Duration) -> Result<bool> {
        tokio::time::sleep(max_wait).await;
        Ok(true)
    }
    /// Whether mailboxes are labels (Gmail) rather than folders.
    fn uses_labels(&self) -> bool {
        false
    }
}

#[async_trait]
pub trait CalendarProvider: Send + Sync {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()>;
    async fn apply(&self, ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome>;
}

/// The account's sign-in predates access to contacts (OAuth scope missing).
/// Contacts sync is skipped until the user signs in again; it is not an error.
#[derive(Debug)]
pub struct ContactsScopeMissing;

impl std::fmt::Display for ContactsScopeMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sign in again to sync contacts")
    }
}

impl std::error::Error for ContactsScopeMissing {}

#[async_trait]
pub trait ContactsProvider: Send + Sync {
    /// Bring address books and their cards up to date (see [`SyncCtx::put_card`]).
    async fn sync(&self, ctx: &SyncCtx) -> Result<()>;
    async fn apply(&self, ctx: &SyncCtx, op: &ContactOp) -> Result<ApplyOutcome>;
    /// The server's current copy of one card as (etag, vCard), or None if it
    /// is gone. Used to merge after a conflict.
    async fn fetch(&self, ctx: &SyncCtx, book_remote_id: &str, card_remote_id: &str) -> Result<Option<(Option<String>, String)>>;
}

pub struct Providers {
    pub mail: Option<Arc<dyn MailProvider>>,
    pub calendar: Option<Arc<dyn CalendarProvider>>,
    pub contacts: Option<Arc<dyn ContactsProvider>>,
}

/// Builds provider instances for an account. Supplied by the host so the
/// core does not depend on protocol crates.
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    fn create(&self, account: &Account, creds: CredentialHandle) -> Result<Providers>;
    /// Start an OAuth flow for a provider kind; returns the URL to open in a
    /// browser. `done` is called once the user finishes (or abandons) sign-in.
    fn oauth_start(&self, _kind: ProviderKind, _done: OAuthDone) -> Result<String> {
        anyhow::bail!("OAuth is not configured for this provider")
    }
    /// RFC 8058 one-click unsubscribe: POST `List-Unsubscribe=One-Click` to an
    /// https URL. Here because the core has no HTTP client of its own.
    async fn one_click_unsubscribe(&self, _url: &str) -> Result<()> {
        anyhow::bail!("unsubscribing is not supported by this host")
    }
    /// The service that looks up places for event locations, if the host has one.
    fn places(&self) -> Option<Arc<dyn PlaceSearch>> {
        None
    }
}

/// Looks up places as the user types an event's location (0050). The query
/// leaves the device, so the engine only asks when `look_up_places` is on.
#[async_trait]
pub trait PlaceSearch: Send + Sync {
    /// Shown in settings: "OpenStreetMap", "Apple Maps".
    fn name(&self) -> &'static str;
    async fn suggest(&self, query: &str) -> Result<Vec<PlaceSuggestion>>;
}

/// Completion callback for an OAuth sign-in: the new account's config and credential.
pub type OAuthDone = Box<dyn FnOnce(Result<(AccountConfig, Credential)>) + Send>;

/// Access to an account's credential, with write-back for refreshed tokens.
#[derive(Clone)]
pub struct CredentialHandle {
    pub account_id: String,
    secrets: Arc<dyn SecretStore>,
}

impl CredentialHandle {
    pub fn new(account_id: String, secrets: Arc<dyn SecretStore>) -> Self {
        Self { account_id, secrets }
    }
    pub fn get(&self) -> Result<Credential> {
        self.secrets
            .get(&account_key(&self.account_id))?
            .ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::Auth, "no credential stored for account")))
    }
    pub fn set(&self, c: &Credential) -> Result<()> {
        self.secrets.set(&account_key(&self.account_id), c)
    }
}
