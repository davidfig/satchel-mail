//! IMAP4rev1/rev2 mail provider with SMTP submission.
//!
//! Remote message identity is `<uidvalidity>:<uid>:<mailbox>` because UIDs
//! are per-mailbox. Moves we perform are followed by a Message-ID search in
//! the target mailbox so the local message keeps its id.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_imap::types::{Fetch, Flag, NameAttribute};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CredentialHandle, MailProvider, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::net::{MaybeTls, tcp, tls_wrap};

type Session = async_imap::Session<MaybeTls>;

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
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MailboxCursor {
    uidvalidity: u32,
    uidnext: u32,
    modseq: Option<u64>,
}

pub fn remote_id(uidvalidity: u32, uid: u32, mailbox: &str) -> String {
    format!("{uidvalidity}:{uid}:{mailbox}")
}

pub fn parse_remote_id(s: &str) -> Option<(u32, u32, &str)> {
    let mut it = s.splitn(3, ':');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?))
}

/// Decode IMAP modified UTF-7 (RFC 3501 §5.1.3).
pub fn decode_mutf7(s: &str) -> String {
    use base64::Engine as _;
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('-') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let enc = &after[..j];
        if enc.is_empty() {
            out.push('&');
        } else {
            let b64 = enc.replace(',', "/");
            let pad = (4 - b64.len() % 4) % 4;
            let b64 = format!("{b64}{}", "=".repeat(pad));
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => out.push_str(&rest[i..i + 2 + j]),
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

fn role_for(name: &str, attrs: &[NameAttribute<'_>]) -> MailboxRole {
    for a in attrs {
        match a {
            NameAttribute::Sent => return MailboxRole::Sent,
            NameAttribute::Drafts => return MailboxRole::Drafts,
            NameAttribute::Trash => return MailboxRole::Trash,
            NameAttribute::Junk => return MailboxRole::Junk,
            NameAttribute::Archive => return MailboxRole::Archive,
            NameAttribute::All => return MailboxRole::All,
            NameAttribute::Flagged => return MailboxRole::Flagged,
            _ => {}
        }
    }
    let leaf = name.rsplit(['/', '.']).next().unwrap_or(name).to_lowercase();
    if name.eq_ignore_ascii_case("INBOX") {
        return MailboxRole::Inbox;
    }
    match leaf.as_str() {
        "sent" | "sent items" | "sent mail" | "sent messages" => MailboxRole::Sent,
        "drafts" | "draft" => MailboxRole::Drafts,
        "trash" | "deleted items" | "deleted messages" | "bin" => MailboxRole::Trash,
        "junk" | "spam" | "junk mail" | "junk e-mail" | "bulk mail" => MailboxRole::Junk,
        "archive" | "archives" => MailboxRole::Archive,
        _ => MailboxRole::Custom,
    }
}

fn flags_of(f: &Fetch) -> (Flags, bool) {
    let mut out = Flags::default();
    let mut deleted = false;
    for fl in f.flags() {
        match fl {
            Flag::Seen => out.seen = true,
            Flag::Answered => out.answered = true,
            Flag::Flagged => out.flagged = true,
            Flag::Draft => out.draft = true,
            Flag::Deleted => deleted = true,
            _ => {}
        }
    }
    (out, deleted)
}

fn uid_set(uids: &[u32]) -> String {
    let mut v = uids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut parts = vec![];
    let mut i = 0;
    while i < v.len() {
        let start = v[i];
        let mut end = start;
        while i + 1 < v.len() && v[i + 1] == end + 1 {
            i += 1;
            end = v[i];
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    parts.join(",")
}

fn imap_date(d: DateTime<Utc>) -> String {
    d.format("%d-%b-%Y").to_string()
}

impl ImapProvider {
    pub fn new(cfg: ImapSettings, creds: CredentialHandle) -> Self {
        Self { cfg, creds, session: Mutex::new(None), caps: Mutex::new(HashSet::new()) }
    }

    fn password(&self) -> Result<String> {
        match self.creds.get()? {
            Credential::Password { password } => Ok(password),
            Credential::OAuth { access_token, .. } => Ok(access_token),
        }
    }

    async fn connect(&self) -> Result<Session> {
        let c = &self.cfg;
        let stream = tcp(&c.host, c.port).await?;
        let mut client = match c.security {
            Security::Tls => {
                let tls = tls_wrap(stream, &c.host, c.allow_invalid_certs).await?;
                let mut cl = async_imap::Client::new(MaybeTls::Tls(Box::new(tls)));
                cl.read_response().await?.context("server closed connection before greeting")?;
                cl
            }
            Security::StartTls => {
                let mut plain = async_imap::Client::new(MaybeTls::Plain(stream));
                plain.read_response().await?.context("server closed connection before greeting")?;
                plain.run_command_and_check_ok("STARTTLS", None).await.context("STARTTLS rejected")?;
                let MaybeTls::Plain(inner) = plain.into_inner() else { unreachable!() };
                let tls = tls_wrap(inner, &c.host, c.allow_invalid_certs).await?;
                async_imap::Client::new(MaybeTls::Tls(Box::new(tls)))
            }
            Security::None => {
                let mut cl = async_imap::Client::new(MaybeTls::Plain(stream));
                cl.read_response().await?.context("server closed connection before greeting")?;
                cl
            }
        };
        let _ = &mut client;
        let password = self.password()?;
        let mut session = tokio::time::timeout(Duration::from_secs(30), client.login(&c.username, &password))
            .await
            .map_err(|_| anyhow!("IMAP login timed out"))?
            .map_err(|(e, _)| anyhow::Error::new(ApiError::new(ErrorCode::Auth, format!("IMAP login failed: {e}"))))?;
        let caps = session.capabilities().await?;
        let mut set = HashSet::new();
        for c in caps.iter() {
            if let async_imap::types::Capability::Atom(a) = c {
                set.insert(a.to_ascii_uppercase());
            }
        }
        if set.contains("ENABLE") && set.contains("CONDSTORE") {
            let _ = session.run_command_and_check_ok("ENABLE CONDSTORE").await;
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

    async fn list_mailboxes(&self, s: &mut Session) -> Result<Vec<(RemoteMailbox, bool)>> {
        let names: Vec<async_imap::types::Name> = s.list(Some(""), Some("*")).await?.try_collect().await?;
        let mut out = vec![];
        for n in names {
            let selectable = !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect))
                && !n.attributes().iter().any(|a| matches!(a, NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent")));
            let raw = n.name().to_string();
            let decoded = decode_mutf7(&raw);
            let delim = n.delimiter().unwrap_or("/");
            let leaf = decoded.rsplit(delim).next().unwrap_or(&decoded).to_string();
            let role = role_for(&decoded, n.attributes());
            let display = if role == MailboxRole::Inbox { "Inbox".to_string() } else { leaf };
            out.push((
                RemoteMailbox { remote_id: raw, name: display, path: decoded.replace(delim, "/"), role, is_label: false },
                selectable,
            ));
        }
        Ok(out)
    }

    async fn sync_mailbox(&self, s: &mut Session, ctx: &SyncCtx, mb: &RemoteMailbox) -> Result<()> {
        let condstore = self.has_cap("CONDSTORE").await;
        let sel = if condstore { s.select_condstore(&mb.remote_id).await? } else { s.select(&mb.remote_id).await? };
        let uv = sel.uid_validity.unwrap_or(0);
        let key = format!("imap:{}", mb.remote_id);
        let cursor: Option<MailboxCursor> = ctx.cursor(&key)?.and_then(|c| serde_json::from_str(&c).ok());
        let local_mb = ctx.mailbox_local_id(&mb.remote_id)?.context("mailbox not stored")?;
        let full = cursor.as_ref().is_none_or(|c| c.uidvalidity != uv);
        let unchanged = !full
            && cursor.as_ref().is_some_and(|c| sel.highest_modseq.is_some() && c.modseq == sel.highest_modseq && Some(c.uidnext) == sel.uid_next);
        if unchanged {
            return Ok(());
        }

        // Local view of this mailbox.
        let mut local: HashMap<u32, (String, Flags)> =
            ctx.store.mailbox_uids(&local_mb)?.into_iter().map(|(u, id, f)| (u, (id, f))).collect();
        if full && cursor.is_some() {
            // UIDVALIDITY changed: every UID is meaningless now.
            for (_, (id, _)) in local.drain() {
                ctx.store.remove_from_mailbox(&id, &local_mb)?;
            }
        }

        // Flags (all, or just changes since the last modseq).
        let since = (!full).then(|| cursor.as_ref().and_then(|c| c.modseq)).flatten().filter(|_| condstore);
        let query = match since {
            Some(m) => format!("(UID FLAGS) (CHANGEDSINCE {m})"),
            None => "(UID FLAGS)".to_string(),
        };
        let mut remote_flags: HashMap<u32, Flags> = HashMap::new();
        if sel.exists > 0 {
            // Stream: each response holds its read buffer, so collecting 100k of
            // them first would keep hundreds of MB alive.
            let mut stream = s.uid_fetch("1:*", &query).await?;
            while let Some(f) = stream.try_next().await? {
                let Some(uid) = f.uid else { continue };
                let (flags, deleted) = flags_of(&f);
                if !deleted {
                    remote_flags.insert(uid, flags);
                }
            }
            drop(stream);
        }
        // Full UID list for deletion detection.
        let all_uids: HashSet<u32> = if since.is_some() {
            if sel.exists == 0 { HashSet::new() } else { s.uid_search("ALL").await? }
        } else {
            remote_flags.keys().copied().collect()
        };

        for (uid, (id, _)) in local.iter() {
            if !all_uids.contains(uid) {
                ctx.store.remove_from_mailbox(id, &local_mb)?;
            }
        }
        for (uid, flags) in &remote_flags {
            if let Some((id, lf)) = local.get(uid)
                && lf != flags {
                    ctx.store.set_flags(id, flags)?;
                }
        }
        let mut new_uids: Vec<u32> = all_uids.iter().filter(|u| !local.contains_key(u)).copied().collect();
        new_uids.sort_unstable_by(|a, b| b.cmp(a));
        if !new_uids.is_empty() {
            let window = Utc::now() - chrono::Duration::days(ctx.settings.initial_sync_days as i64);
            let recent: HashSet<u32> = s.uid_search(format!("SINCE {}", imap_date(window))).await?.into_iter().collect();
            let (full_body, headers_only): (Vec<u32>, Vec<u32>) = new_uids.iter().partition(|u| recent.contains(u));
            let total = new_uids.len();
            let mut done = 0;
            for chunk in full_body.chunks(25) {
                ctx.progress(format!("{}: downloading {}/{}", mb.name, done, total));
                self.fetch_into(s, ctx, mb, uv, chunk, true).await?;
                done += chunk.len();
            }
            for chunk in headers_only.chunks(250) {
                ctx.progress(format!("{}: indexing {}/{}", mb.name, done, total));
                self.fetch_into(s, ctx, mb, uv, chunk, false).await?;
                done += chunk.len();
            }
        }
        // Optimistic placements the server never confirmed (e.g. a move whose copy we
        // could not re-identify) are superseded by the real copies fetched above.
        if !ctx.store.has_pending_ops(ctx.account_id())? {
            for id in ctx.store.mailbox_unconfirmed(&local_mb)? {
                ctx.store.remove_from_mailbox(&id, &local_mb)?;
            }
        }
        let c = MailboxCursor { uidvalidity: uv, uidnext: sel.uid_next.unwrap_or(0), modseq: sel.highest_modseq };
        ctx.set_cursor(&key, &serde_json::to_string(&c)?)?;
        Ok(())
    }

    async fn fetch_into(&self, s: &mut Session, ctx: &SyncCtx, mb: &RemoteMailbox, uv: u32, uids: &[u32], body: bool) -> Result<()> {
        let q = if body { "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])" } else { "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])" };
        let mut stream = s.uid_fetch(uid_set(uids), q).await?;
        while let Some(f) = stream.try_next().await? {
            let Some(uid) = f.uid else { continue };
            let (flags, deleted) = flags_of(&f);
            if deleted {
                continue;
            }
            let rid = remote_id(uv, uid, &mb.remote_id);
            let internal = f.internal_date().map(|d| d.with_timezone(&Utc));
            let mbs = [(mb.remote_id.clone(), Some(uid))];
            if body
                && let Some(raw) = f.body() {
                    ctx.put_raw_message(&rid, None, raw, flags, &mbs, internal)?;
                    continue;
                }
            if let Some(hdr) = f.header().or(f.body()) {
                let p = satchel_mime::parse(hdr).unwrap_or_default();
                let rec = MessageRecord {
                    remote_id: rid,
                    message_id_hdr: p.message_id,
                    in_reply_to: p.in_reply_to,
                    references: p.references,
                    subject: p.subject,
                    from: p.from,
                    to: p.to,
                    cc: p.cc,
                    bcc: p.bcc,
                    reply_to: p.reply_to,
                    date: p.date.or(internal).unwrap_or_else(Utc::now),
                    flags,
                    size: f.size.unwrap_or(0) as u64,
                    ..Default::default()
                };
                ctx.put_record(rec, &mbs)?;
            }
        }
        Ok(())
    }

    /// Group message refs by (mailbox remote id) -> uids.
    fn by_mailbox(messages: &[MessageRef]) -> HashMap<String, Vec<(u32, String)>> {
        let mut out: HashMap<String, Vec<(u32, String)>> = HashMap::new();
        for m in messages {
            if let Some((_, uid, mb)) = parse_remote_id(&m.remote_id) {
                out.entry(mb.to_string()).or_default().push((uid, m.id.clone()));
                continue;
            }
            for (_, rmb, uid) in &m.mailboxes {
                if let Some(u) = uid {
                    out.entry(rmb.clone()).or_default().push((*u, m.id.clone()));
                }
            }
        }
        out
    }

    /// UIDNEXT of a mailbox (taken before a MOVE/APPEND so we can find the new copies).
    async fn uid_next(&self, s: &mut Session, mailbox: &str) -> Result<(u32, u32)> {
        let sel = s.select(mailbox).await?;
        Ok((sel.uid_validity.unwrap_or(0), sel.uid_next.unwrap_or(1)))
    }

    /// Message-ID -> (uidvalidity, uid) for messages that arrived at or after `from_uid`.
    async fn locate_new(&self, s: &mut Session, mailbox: &str, from_uid: u32) -> Result<HashMap<String, (u32, u32)>> {
        let sel = s.select(mailbox).await?;
        let uv = sel.uid_validity.unwrap_or(0);
        let mut out = HashMap::new();
        if sel.exists == 0 {
            return Ok(out);
        }
        let fetches: Vec<Fetch> = s.uid_fetch(format!("{from_uid}:*"), "(UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])").await?.try_collect().await?;
        for f in fetches {
            let (Some(uid), Some(h)) = (f.uid, f.header().or(f.body())) else { continue };
            if uid < from_uid {
                continue; // "n:*" always returns the last message even if older
            }
            if let Some(mid) = satchel_mime::parse(h).and_then(|p| p.message_id) {
                out.insert(mid, (uv, uid));
            }
        }
        Ok(out)
    }

    async fn expunge_uids(&self, s: &mut Session, mailbox: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        s.select(mailbox).await?;
        let set = uid_set(uids);
        let _: Vec<Fetch> = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        if self.has_cap("UIDPLUS").await {
            let _: Vec<u32> = s.uid_expunge(&set).await?.try_collect().await?;
        } else {
            let _: Vec<u32> = s.expunge().await?.try_collect().await?;
        }
        Ok(())
    }

    async fn smtp_send(&self, raw: &[u8], from: &str, rcpts: &[String]) -> Result<()> {
        use lettre::address::Envelope;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::transport::smtp::client::{Tls, TlsParameters};
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
        let c = &self.cfg;
        let tls = TlsParameters::builder(c.smtp_host.clone())
            .dangerous_accept_invalid_certs(c.allow_invalid_certs)
            .dangerous_accept_invalid_hostnames(c.allow_invalid_certs)
            .build_rustls()?;
        let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(c.smtp_host.clone())
            .port(c.smtp_port)
            .timeout(Some(Duration::from_secs(60)))
            .credentials(Credentials::new(c.username.clone(), self.password()?));
        b = match c.smtp_security {
            Security::Tls => b.tls(Tls::Wrapper(tls)),
            Security::StartTls => b.tls(Tls::Required(tls)),
            Security::None => b.tls(Tls::None),
        };
        let t = b.build();
        let env = Envelope::new(
            Some(from.parse().map_err(|e| anyhow!("bad from address {from}: {e}"))?),
            rcpts.iter().map(|r| r.parse().map_err(|e| anyhow!("bad recipient {r}: {e}"))).collect::<Result<Vec<_>>>()?,
        )?;
        t.send_raw(&env, raw).await.map_err(|e| {
            let msg = format!("SMTP: {e}");
            let code = if e.is_permanent() { ErrorCode::InvalidInput } else if e.is_transient() { ErrorCode::Network } else { ErrorCode::Internal };
            anyhow::Error::new(ApiError::new(code, msg))
        })?;
        Ok(())
    }

    fn server_saves_sent(&self) -> bool {
        let h = self.cfg.smtp_host.to_lowercase();
        h.ends_with("gmail.com") || h.ends_with("googlemail.com") || h.contains("office365.com") || h.contains("outlook.com")
    }

    async fn apply_inner(&self, s: &mut Session, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, answered } => {
                for (mb, items) in Self::by_mailbox(messages) {
                    s.select(&mb).await?;
                    let set = uid_set(&items.iter().map(|x| x.0).collect::<Vec<_>>());
                    let mut add = vec![];
                    let mut rem = vec![];
                    for (v, flag) in [(seen, "\\Seen"), (flagged, "\\Flagged"), (answered, "\\Answered")] {
                        match v {
                            Some(true) => add.push(flag),
                            Some(false) => rem.push(flag),
                            None => {}
                        }
                    }
                    if !add.is_empty() {
                        let _: Vec<Fetch> = s.uid_store(&set, format!("+FLAGS.SILENT ({})", add.join(" "))).await?.try_collect().await?;
                    }
                    if !rem.is_empty() {
                        let _: Vec<Fetch> = s.uid_store(&set, format!("-FLAGS.SILENT ({})", rem.join(" "))).await?.try_collect().await?;
                    }
                }
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                let has_move = self.has_cap("MOVE").await;
                let (_, next_before) = self.uid_next(s, to_remote_mailbox).await?;
                for (mb, items) in Self::by_mailbox(messages) {
                    if &mb == to_remote_mailbox {
                        continue;
                    }
                    s.select(&mb).await?;
                    let uids: Vec<u32> = items.iter().map(|x| x.0).collect();
                    let set = uid_set(&uids);
                    if has_move {
                        s.uid_mv(&set, to_remote_mailbox).await?;
                    } else {
                        s.uid_copy(&set, to_remote_mailbox).await?;
                        self.expunge_uids(s, &mb, &uids).await?;
                    }
                }
                // Re-identify moved messages in the target mailbox so local ids survive.
                let arrived = self.locate_new(s, to_remote_mailbox, next_before).await?;
                for m in messages {
                    if let Some((uv, uid)) = m.message_id_hdr.as_ref().and_then(|mid| arrived.get(mid)) {
                        out.rekeyed.push((m.id.clone(), remote_id(*uv, *uid, to_remote_mailbox), vec![(to_remote_mailbox.clone(), Some(*uid))]));
                    }
                }
            }
            ResolvedOp::Labels { .. } => bail!("IMAP accounts use folders, not labels"),
            ResolvedOp::Expunge { messages } => {
                for (mb, items) in Self::by_mailbox(messages) {
                    self.expunge_uids(s, &mb, &items.iter().map(|x| x.0).collect::<Vec<_>>()).await?;
                }
            }
            ResolvedOp::SaveDraft { raw, replaces, drafts_remote_mailbox } => {
                let mb = drafts_remote_mailbox.clone().unwrap_or_else(|| "Drafts".into());
                let (_, next_before) = self.uid_next(s, &mb).await?;
                s.append(&mb, Some("(\\Draft \\Seen)"), None, raw).await?;
                if let Some(old) = replaces {
                    for (omb, items) in Self::by_mailbox(std::slice::from_ref(old)) {
                        self.expunge_uids(s, &omb, &items.iter().map(|x| x.0).collect::<Vec<_>>()).await?;
                    }
                }
                let parsed = satchel_mime::parse(raw).unwrap_or_default();
                if let Some(mid) = parsed.message_id
                    && let Some((uv, uid)) = self.locate_new(s, &mb, next_before).await?.get(&mid) {
                        out.stored_remote_id = Some(remote_id(*uv, *uid, &mb));
                    }
            }
            ResolvedOp::DeleteDraft { message } => {
                for (mb, items) in Self::by_mailbox(std::slice::from_ref(message)) {
                    self.expunge_uids(s, &mb, &items.iter().map(|x| x.0).collect::<Vec<_>>()).await?;
                }
            }
            ResolvedOp::Send { raw, from, rcpts, sent_remote_mailbox, replaces_draft, .. } => {
                self.smtp_send(raw, from, rcpts).await?;
                // After this point the mail is out: never fail the op (it would resend).
                if !self.server_saves_sent()
                    && let Some(sent) = sent_remote_mailbox
                        && let Err(e) = s.append(sent, Some("(\\Seen)"), None, raw).await {
                            warn!("could not save to Sent: {e}");
                        }
                if let Some(d) = replaces_draft {
                    for (mb, items) in Self::by_mailbox(std::slice::from_ref(d)) {
                        if let Err(e) = self.expunge_uids(s, &mb, &items.iter().map(|x| x.0).collect::<Vec<_>>()).await {
                            warn!("could not remove draft: {e}");
                        }
                    }
                }
            }
        }
        Ok(out)
    }
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
            for (mb, _) in order {
                if let Err(e) = self.sync_mailbox(&mut s, ctx, mb).await {
                    warn!(mailbox = %mb.path, "sync failed: {e:#}");
                    return Err(e);
                }
            }
            Ok(())
        }
        .await;
        if r.is_ok() {
            self.release(s).await;
        }
        r
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let (mb, uid) = match parse_remote_id(&msg.remote_id) {
            Some((_, uid, mb)) => (mb.to_string(), uid),
            None => {
                let (_, rmb, uid) = msg.mailboxes.iter().find(|(_, _, u)| u.is_some()).context("message has no IMAP location")?;
                (rmb.clone(), uid.unwrap())
            }
        };
        let mut s = self.session().await?;
        s.examine(&mb).await?;
        let fetches: Vec<Fetch> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
        let raw = fetches.into_iter().find_map(|f| f.body().map(|b| b.to_vec()));
        self.release(s).await;
        raw.ok_or_else(|| anyhow::Error::new(ApiError::new(ErrorCode::NotFound, "message no longer on server")))
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
        let mut s = self.connect().await?;
        s.select("INBOX").await?;
        let mut idle = s.idle();
        idle.init().await?;
        let (fut, _stop) = idle.wait_with_timeout(max_wait);
        let res = fut.await;
        let changed = matches!(res, Ok(async_imap::extensions::idle::IdleResponse::NewData(_)));
        debug!("IDLE returned (changed={changed})");
        if let Ok(mut s) = idle.done().await {
            let _ = s.logout().await;
        }
        res?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_mutf7("Caf&AOk-"), "Café");
        assert_eq!(decode_mutf7("A&-B"), "A&B");
    }

    #[test]
    fn uidsets() {
        assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10]), "1:3,5,9:10");
    }

    #[test]
    fn remote_ids() {
        let r = remote_id(7, 42, "Folder:With:Colons");
        assert_eq!(parse_remote_id(&r), Some((7, 42, "Folder:With:Colons")));
    }
}
