//! The contract between the core and provider adapters.
//!
//! Providers do protocol work only. They write what they find through
//! [`SyncCtx`], which owns all persistence, threading, indexing and events.

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
    Delete { calendar_remote_id: String, event_remote_id: String, etag: Option<String> },
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    /// Verify credentials and connectivity.
    async fn test(&self) -> Result<()>;
    /// Bring the local store up to date (delta when possible).
    async fn sync(&self, ctx: &SyncCtx) -> Result<()>;
    /// Fetch a full raw message.
    async fn fetch_raw(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>>;
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

pub struct Providers {
    pub mail: Option<Arc<dyn MailProvider>>,
    pub calendar: Option<Arc<dyn CalendarProvider>>,
}

/// Builds provider instances for an account. Supplied by the host so the
/// core does not depend on protocol crates.
pub trait ProviderFactory: Send + Sync {
    fn create(&self, account: &Account, creds: CredentialHandle) -> Result<Providers>;
    /// Start an OAuth flow for a provider kind; returns the URL to open in a
    /// browser. `done` is called once the user finishes (or abandons) sign-in.
    fn oauth_start(&self, _kind: ProviderKind, _done: OAuthDone) -> Result<String> {
        anyhow::bail!("OAuth is not configured for this provider")
    }
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

/// Everything a provider needs to persist sync results.
#[derive(Clone)]
pub struct SyncCtx {
    pub account: Account,
    pub store: Arc<Store>,
    pub blobs: Arc<dyn BlobStore>,
    pub settings: Settings,
    pub(crate) events: tokio::sync::broadcast::Sender<CoreEvent>,
}

impl SyncCtx {
    pub fn account_id(&self) -> &str {
        &self.account.id
    }

    pub fn progress(&self, detail: impl Into<String>) {
        let st = SyncStatus { state: SyncState::Syncing, last_success: self.account.sync.last_success, error: None, detail: Some(detail.into()) };
        let _ = self.store.set_sync_status(&self.account.id, &st);
        let _ = self.events.send(CoreEvent::SyncStatus { account_id: self.account.id.clone(), status: st });
    }

    pub fn cursor(&self, key: &str) -> Result<Option<String>> {
        self.store.cursor(&self.account.id, key)
    }

    pub fn set_cursor(&self, key: &str, value: &str) -> Result<()> {
        self.store.set_cursor(&self.account.id, key, value)
    }

    /// Record the full mailbox list; prunes ones that disappeared. Returns
    /// (remote id -> local id).
    pub fn set_mailboxes(&self, mbs: &[RemoteMailbox]) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for m in mbs {
            let id = self.store.upsert_mailbox(&self.account.id, &m.remote_id, &m.name, &m.path, m.role, m.is_label)?;
            map.insert(m.remote_id.clone(), id);
        }
        let keep: Vec<String> = mbs.iter().map(|m| m.remote_id.clone()).collect();
        self.store.prune_mailboxes(&self.account.id, &keep)?;
        let _ = self.events.send(CoreEvent::MailboxesChanged { account_id: self.account.id.clone() });
        Ok(map)
    }

    pub fn mailbox_local_id(&self, remote_id: &str) -> Result<Option<String>> {
        Ok(self.store.mailbox_by_remote(&self.account.id, remote_id)?.map(|m| m.id))
    }

    /// Store a message from raw RFC 822 bytes. `mailboxes` are remote mailbox ids with optional UIDs.
    pub fn put_raw_message(
        &self,
        remote_id: &str,
        remote_thread_id: Option<&str>,
        raw: &[u8],
        flags: Flags,
        mailboxes: &[(String, Option<u32>)],
        internal_date: Option<DateTime<Utc>>,
    ) -> Result<String> {
        let parsed = satchel_mime::parse(raw).unwrap_or_default();
        let hash = self.blobs.put(raw)?;
        let rec = MessageRecord {
            remote_id: remote_id.to_string(),
            remote_thread_id: remote_thread_id.map(|s| s.to_string()),
            message_id_hdr: parsed.message_id.clone(),
            in_reply_to: parsed.in_reply_to.clone(),
            references: parsed.references.clone(),
            subject: parsed.subject.clone(),
            from: parsed.from.clone(),
            to: parsed.to.clone(),
            cc: parsed.cc.clone(),
            bcc: parsed.bcc.clone(),
            reply_to: parsed.reply_to.clone(),
            date: parsed.date.or(internal_date).unwrap_or_else(Utc::now),
            snippet: parsed.snippet.clone(),
            flags,
            attachments: parsed.attachments.clone(),
            blob_hash: Some(hash),
            size: raw.len() as u64,
        };
        self.put_record(rec, mailboxes)
    }

    /// Store a message from metadata only (body fetched later).
    pub fn put_record(&self, rec: MessageRecord, mailboxes: &[(String, Option<u32>)]) -> Result<String> {
        let local: Vec<(String, Option<u32>)> = mailboxes
            .iter()
            .filter_map(|(r, uid)| self.mailbox_local_id(r).ok().flatten().map(|id| (id, *uid)))
            .collect();
        let (id, _) = self.store.upsert_message(&self.account.id, &rec, &local)?;
        Ok(id)
    }

    pub fn message_by_remote(&self, remote_id: &str) -> Result<Option<String>> {
        self.store.message_id_by_remote(&self.account.id, remote_id)
    }

    pub fn set_flags_remote(&self, remote_id: &str, flags: Flags) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            self.store.set_flags(&id, &flags)?;
        }
        Ok(())
    }

    pub fn set_memberships_remote(&self, remote_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            let local: Vec<(String, Option<u32>)> = mailboxes
                .iter()
                .filter_map(|(r, uid)| self.mailbox_local_id(r).ok().flatten().map(|mid| (mid, *uid)))
                .collect();
            if local.is_empty() {
                self.store.delete_message(&id)?;
            } else {
                self.store.set_memberships(&id, &local)?;
            }
        }
        Ok(())
    }

    pub fn delete_remote(&self, remote_id: &str) -> Result<()> {
        if let Some(id) = self.message_by_remote(remote_id)? {
            self.store.delete_message(&id)?;
        }
        Ok(())
    }

    pub fn messages_changed(&self) {
        let _ = self.store.recount_mailboxes(&self.account.id);
        let _ = self.events.send(CoreEvent::MessagesChanged { account_id: self.account.id.clone() });
    }

    // calendars

    pub fn set_calendars(&self, cals: &[RemoteCalendar]) -> Result<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for c in cals {
            let id = self.store.upsert_calendar(&self.account.id, &c.remote_id, &c.name, c.color.as_deref(), c.read_only)?;
            map.insert(c.remote_id.clone(), id);
        }
        let keep: Vec<String> = cals.iter().map(|c| c.remote_id.clone()).collect();
        self.store.prune_calendars(&self.account.id, &keep)?;
        Ok(map)
    }

    /// Store an event's iCalendar object (all components for one UID/resource).
    pub fn put_event(&self, calendar_id: &str, remote_id: &str, etag: Option<&str>, ics: &str) -> Result<()> {
        let parsed = satchel_ical::parse(ics);
        let Some(master) = parsed.events.iter().find(|e| e.recurrence_id.is_none()).or(parsed.events.first()) else {
            return Ok(());
        };
        let (start, end) = crate::calendar::series_range(&parsed.events);
        self.store.upsert_event(
            calendar_id,
            &self.account.id,
            remote_id,
            etag,
            &master.uid,
            ics,
            &master.summary,
            (start, end),
            master.rrule.is_some(),
            false,
        )?;
        Ok(())
    }

    pub fn delete_event_remote(&self, calendar_id: &str, remote_id: &str) -> Result<()> {
        self.store.delete_event_by_remote(calendar_id, remote_id)
    }

    pub fn calendar_changed(&self) {
        let _ = self.events.send(CoreEvent::CalendarChanged { account_id: self.account.id.clone() });
    }
}
