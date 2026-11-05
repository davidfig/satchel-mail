//! IMAP4rev1/rev2 mail provider with SMTP submission.
//!
//! Remote message identity is `<uidvalidity>:<uid>:<mailbox>` because UIDs
//! are per-mailbox. Moves we perform are followed by a Message-ID search in
//! the target mailbox so the local message keeps its id.

mod apply;
mod names;
mod parts;
mod smtp;
mod sync;
#[cfg(test)]
mod tests;

pub use names::{decode_mutf7, encode_mutf7, parse_remote_id, remote_id};
use names::*;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_imap::types::{Fetch, Flag, NameAttribute};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, priority, CredentialHandle, Download, MailProvider, MaybeSent, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::net::{MaybeTls, Timed, tcp, tls_wrap};

type Session = async_imap::Session<Timed<MaybeTls>>;

/// How long the server may stay silent while we wait on it before the
/// connection counts as dead.
const STALL: Duration = Duration::from_secs(180);

#[derive(Clone)]
pub struct ImapSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub allow_invalid_certs: bool,
}

pub struct ImapProvider {
    cfg: ImapSettings,
    creds: CredentialHandle,
    session: Mutex<Option<Session>>,
    caps: Mutex<HashSet<String>>,
    /// Older mail the last sync found missing, for [`MailProvider::backfill`].
    older: Mutex<Vec<Older>>,
}

/// A mailbox, its UIDVALIDITY, and the UIDs of older mail not fetched yet,
/// newest first.
type Older = (RemoteMailbox, u32, Vec<u32>);

#[derive(Debug, Default, Serialize, Deserialize)]
struct MailboxCursor {
    uidvalidity: u32,
    uidnext: u32,
    modseq: Option<u64>,
    /// Older mail in this mailbox hasn't all been fetched yet (0039).
    #[serde(default)]
    backfill: bool,
}

impl ImapProvider {
    pub fn new(cfg: ImapSettings, creds: CredentialHandle) -> Self {
        Self { cfg, creds, session: Mutex::new(None), caps: Mutex::new(HashSet::new()), older: Mutex::new(vec![]) }
    }

    fn password(&self) -> Result<String> {
        match self.creds.get()? {
            Credential::Password { password } => Ok(password),
            Credential::OAuth { access_token, .. } => Ok(access_token),
        }
    }

    async fn connect(&self) -> Result<Session> {
        self.connect_with(Some(STALL)).await
    }

    /// Connect and sign in; `stall` as in [`Timed`]. The greeting and STARTTLS
    /// are always bounded.
    async fn connect_with(&self, stall: Option<Duration>) -> Result<Session> {
        let c = &self.cfg;
        if c.security == Security::None {
            crate::net::plaintext_allowed(&c.host, "IMAP")?;
        }
        let stream = tcp(&c.host, c.port).await?;
        let greet = Some(stall.unwrap_or(STALL));
        let client = match c.security {
            Security::Tls => {
                let tls = tls_wrap(stream, &c.host, c.allow_invalid_certs).await?;
                let mut cl = async_imap::Client::new(Timed::new(MaybeTls::Tls(Box::new(tls)), greet));
                cl.read_response().await?.context("server closed connection before greeting")?;
                let inner = cl.into_inner().into_inner();
                async_imap::Client::new(Timed::new(inner, stall))
            }
            Security::StartTls => {
                let mut plain = async_imap::Client::new(Timed::new(MaybeTls::Plain(stream), greet));
                plain.read_response().await?.context("server closed connection before greeting")?;
                plain.run_command_and_check_ok("STARTTLS", None).await.context("STARTTLS rejected")?;
                let MaybeTls::Plain(inner) = plain.into_inner().into_inner() else { unreachable!() };
                let tls = tls_wrap(inner, &c.host, c.allow_invalid_certs).await?;
                async_imap::Client::new(Timed::new(MaybeTls::Tls(Box::new(tls)), stall))
            }
            Security::None => {
                let mut cl = async_imap::Client::new(Timed::new(MaybeTls::Plain(stream), greet));
                cl.read_response().await?.context("server closed connection before greeting")?;
                let inner = cl.into_inner().into_inner();
                async_imap::Client::new(Timed::new(inner, stall))
            }
        };
        self.login(client).await
    }

    async fn login(&self, client: async_imap::Client<Timed<MaybeTls>>) -> Result<Session> {
        let c = &self.cfg;
        let password = self.password()?;
        let mut session = tokio::time::timeout(Duration::from_secs(30), client.login(&c.username, &password))
            .await
            .map_err(|_| anyhow!("IMAP login timed out"))?
            .map_err(|(e, _)| anyhow::Error::new(ApiError::new(ErrorCode::Auth, format!("IMAP login failed: {e}"))))?;
        // Bounded even on the IDLE connection, whose stream never times out.
        let caps = tokio::time::timeout(STALL, session.capabilities()).await.map_err(|_| anyhow!("IMAP CAPABILITY timed out"))??;
        let mut set = HashSet::new();
        for c in caps.iter() {
            if let async_imap::types::Capability::Atom(a) = c {
                set.insert(a.to_ascii_uppercase());
            }
        }
        if set.contains("ENABLE") && set.contains("CONDSTORE") {
            // A refusal is fine (we just don't get CONDSTORE); silence isn't.
            tokio::time::timeout(STALL, session.run_command_and_check_ok("ENABLE CONDSTORE")).await.map_err(|_| anyhow!("IMAP ENABLE timed out"))?.ok();
        }
        *self.caps.lock().await = set;
        Ok(session)
    }

    async fn session(&self) -> Result<Session> {
        if let Some(mut s) = self.session.lock().await.take()
            && tokio::time::timeout(Duration::from_secs(10), s.noop()).await.is_ok_and(|r| r.is_ok()) {
                return Ok(s);
            }
        self.connect().await
    }

    async fn release(&self, s: Session) {
        *self.session.lock().await = Some(s);
    }

    async fn has_cap(&self, c: &str) -> bool {
        self.caps.lock().await.contains(c)
    }

    /// The whole message with UID `uid` in the selected mailbox, unless the
    /// server says it's over [`MAX_MESSAGE`].
    async fn fetch_full(s: &mut Session, uid: u32) -> Result<Vec<u8>> {
        if let Some(&size) = sizes(s, &uid.to_string()).await?.get(&uid)
            && size > MAX_MESSAGE
        {
            bail!(ApiError::new(ErrorCode::InvalidInput, format!("this message is too large to open here ({} MB); open it in webmail", size >> 20)));
        }
        let fetches: Vec<Fetch> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
        let raw = fetches.into_iter().find_map(|f| f.body().map(|b| b.to_vec()));
        raw.ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::NotFound, "message no longer on server")))
    }
}

/// Whole messages over this come down as headers only during sync, and
/// their content when opened. async-imap holds a whole literal in memory
/// (up to its own 512 MiB cap), so this keeps sync's memory bounded.
const MAX_SYNC_MESSAGE: u64 = 64 << 20;
/// The largest message we fetch at all, when the user opens it.
const MAX_MESSAGE: u64 = 160 << 20;

/// `RFC822.SIZE` of the messages in `uids` (an IMAP UID set).
async fn sizes(s: &mut Session, uids: &str) -> Result<HashMap<u32, u64>> {
    let fetches: Vec<Fetch> = s.uid_fetch(uids, "(UID RFC822.SIZE)").await?.try_collect().await?;
    Ok(fetches.iter().filter_map(|f| Some((f.uid?, f.size? as u64))).collect())
}

/// A stored message's mailbox and UID.
fn location(msg: &MessageRef) -> Result<(String, u32)> {
    Ok(match parse_remote_id(&msg.remote_id) {
        Some((_, uid, mb)) => (mb.to_string(), uid),
        None => {
            let (_, rmb, uid) = msg.mailboxes.iter().find(|(_, _, u)| u.is_some()).context("message has no IMAP location")?;
            (rmb.clone(), uid.unwrap())
        }
    })
}

#[async_trait]
impl MailProvider for ImapProvider {
    async fn test(&self) -> Result<()> {
        let mut s = self.connect().await?;
        let _ = s.logout().await;
        Ok(())
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let mut s = self.session().await?;
        let r: Result<()> = async {
            let listed = self.list_mailboxes(&mut s).await?;
            let mbs: Vec<RemoteMailbox> = listed.iter().map(|(m, _)| m.clone()).collect();
            ctx.set_mailboxes(&mbs)?;
            // Inbox first so new mail shows up quickly.
            let mut order: Vec<&(RemoteMailbox, bool)> = listed.iter().filter(|(_, sel)| *sel).collect();
            order.sort_by_key(|(m, _)| match m.role {
                MailboxRole::Inbox => 0,
                MailboxRole::Sent => 1,
                MailboxRole::Drafts => 2,
                MailboxRole::Custom | MailboxRole::Archive => 3,
                _ => 4,
            });
            // One folder the server won't open (a shared folder without
            // access, a glitch) must not stop the rest. The account shows an
            // error only when the Inbox fails or the connection itself broke.
            // Recent mail only; older mail follows in `backfill` (0047).
            let mut inbox_err = None;
            let mut older = vec![];
            for (mb, _) in order {
                priority::yield_to_user().await;
                match self.sync_mailbox(&mut s, ctx, mb).await {
                    Ok((uv, uids)) if !uids.is_empty() => older.push((mb.clone(), uv, uids)),
                    Ok(_) => {}
                    Err(e) => {
                        warn!(mailbox = %mb.path, "sync failed: {e:#}");
                        if !matches!(tokio::time::timeout(Duration::from_secs(30), s.noop()).await, Ok(Ok(_))) {
                            return Err(e);
                        }
                        if mb.role == MailboxRole::Inbox {
                            inbox_err = Some(e);
                        }
                    }
                }
            }
            *self.older.lock().await = older;
            inbox_err.map_or(Ok(()), Err)
        }
        .await;
        if r.is_ok() {
            self.release(s).await;
        }
        r
    }

    async fn backfill(&self, ctx: &SyncCtx, until: std::time::Instant) -> Result<()> {
        let older = std::mem::take(&mut *self.older.lock().await);
        if older.is_empty() {
            return Ok(());
        }
        let mut s = self.session().await?;
        let r = self.fetch_older(&mut s, ctx, older, until).await;
        if r.is_ok() {
            self.release(s).await;
        }
        r
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let (mb, uid) = location(msg)?;
        let mut s = self.session().await?;
        s.examine(&mb).await?;
        let raw = Self::fetch_full(&mut s, uid).await;
        self.release(s).await;
        raw
    }

    async fn fetch_without_attachments(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Download> {
        let (mb, uid) = location(msg)?;
        let mut s = self.session().await?;
        s.examine(&mb).await?;
        let r = async {
            if let Some(plan) = self.plans(&mut s, &uid.to_string()).await?.remove(&uid)
                && let Some(slim) = self.fetch_slim(&mut s, uid, &plan).await? {
                    return Ok(Download::Slim(slim));
                }
            Ok(Download::Full(Self::fetch_full(&mut s, uid).await?))
        }
        .await;
        self.release(s).await;
        r
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut s = self.session().await?;
        let r = self.apply_inner(&mut s, op).await;
        if r.is_ok() || matches!(op, ResolvedOp::Send { .. }) {
            self.release(s).await;
        }
        r
    }

    async fn wait_for_changes(&self, _ctx: &SyncCtx, max_wait: Duration) -> Result<bool> {
        if !self.has_cap("IDLE").await {
            tokio::time::sleep(max_wait).await;
            return Ok(true);
        }
        // IDLE waits quietly on purpose, so this connection has no stall
        // timeout; the wait itself is bounded, and so is everything after it.
        let mut s = self.connect_with(None).await?;
        tokio::time::timeout(STALL, s.select("INBOX")).await.map_err(|_| anyhow!("IMAP SELECT timed out"))??;
        let mut idle = s.idle();
        tokio::time::timeout(STALL, idle.init()).await.map_err(|_| anyhow!("IMAP IDLE timed out"))??;
        let (fut, _stop) = idle.wait_with_timeout(max_wait);
        let res = fut.await;
        let changed = matches!(res, Ok(async_imap::extensions::idle::IdleResponse::NewData(_)));
        debug!("IDLE returned (changed={changed})");
        if let Ok(Ok(mut s)) = tokio::time::timeout(Duration::from_secs(30), idle.done()).await {
            let _ = tokio::time::timeout(Duration::from_secs(10), s.logout()).await;
        }
        res?;
        Ok(true)
    }
}
