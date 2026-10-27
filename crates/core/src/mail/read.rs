//! Reading mail: thread lists, messages, address suggestions.

use std::collections::HashSet;

use anyhow::Result;
use base64::Engine as _;
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::{Caller, Engine, Mode};
use crate::error::{err, forbidden, not_found};
use crate::store::{StoredMessage, ThreadFilter};

impl Engine {
    pub(crate) fn list_threads(&self, caller: &Caller, q: &ThreadQuery) -> Result<Vec<ThreadSummary>> {
        let f = self.thread_filter(caller, q)?;
        self.store.list_threads(&f)
    }

    pub(crate) fn thread_filter(&self, caller: &Caller, q: &ThreadQuery) -> Result<ThreadFilter> {
        let mut f = ThreadFilter {
            mailbox_id: q.mailbox_id.clone(),
            role: q.role,
            before: q.before,
            before_id: q.before_id.clone(),
            offset: q.offset.unwrap_or(0),
            limit: q.limit.unwrap_or(50).min(500),
            ..Default::default()
        };
        if let Some(mb) = &q.mailbox_id {
            let m = self.store.get_mailbox(mb)?.ok_or_else(|| not_found("mailbox"))?;
            self.check_account(caller, &m.account_id)?;
        }
        let mut accounts = q.account_id.clone().map(|a| vec![a]);
        if let Some(allowed) = self.allowed_accounts(caller) {
            accounts = Some(match accounts {
                Some(a) => a.into_iter().filter(|x| allowed.contains(x)).collect(),
                None => allowed,
            });
        }
        f.account_ids = accounts;
        f.after = self.cutoff(caller);
        if let Some(s) = q.search.as_deref().filter(|s| !s.trim().is_empty()) {
            let p = crate::mail::search::parse_query(s);
            f.from = p.from;
            f.to = p.to;
            f.with = p.with;
            f.subject = p.subject;
            f.has_attachment = p.has_attachment;
            f.unread = p.unread;
            f.flagged = p.flagged;
            f.after = match (f.after, p.after) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            f.msg_before = p.before;
            if let Some(m) = p.in_mailbox {
                f.in_mailbox_names = Some(vec![m]);
            } else if q.mailbox_id.is_none() && q.role.is_none() {
                f.exclude_trash_junk = true;
            }
            f.fts = crate::mail::search::fts_query(&p.text);
        }
        Ok(f)
    }

    pub(crate) async fn raw_for(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        if let Some(h) = &m.blob_hash
            && let Some(b) = self.blobs.get(h)? {
                return Ok(Some(b));
            }
        if self.mode != Mode::Full {
            return Ok(None);
        }
        let providers = self.providers_for(&m.summary.account_id)?;
        let Some(mail) = providers.mail.clone() else { return Ok(None) };
        let ctx = self.ctx(&m.summary.account_id)?;
        match tokio::time::timeout(std::time::Duration::from_secs(30), self.ensure_body(&ctx, mail.as_ref(), id)).await {
            Ok(r) => r,
            Err(_) => Err(err(ErrorCode::Network, "timed out fetching message")),
        }
    }

    /// Like [`Self::raw_for`], but with attachment contents: a stored copy
    /// that leaves them out is replaced by the full message from the server.
    pub(crate) async fn full_raw_for(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        if !(m.partial && m.blob_hash.is_some()) || self.mode != Mode::Full {
            return self.raw_for(id).await;
        }
        let providers = self.providers_for(&m.summary.account_id)?;
        let Some(mail) = providers.mail.clone() else { return self.raw_for(id).await };
        let ctx = self.ctx(&m.summary.account_id)?;
        match tokio::time::timeout(std::time::Duration::from_secs(120), self.ensure_full(&ctx, mail.as_ref(), id)).await {
            Ok(r) => r.map(Some),
            Err(_) => Err(err(ErrorCode::Network, "timed out fetching attachments")),
        }
    }

    pub(crate) async fn message_detail(&self, id: &str, force_remote: bool) -> Result<MessageDetail> {
        let mut m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        // Inline images too big to keep in the stored copy come with the full message.
        let big_inline = |m: &StoredMessage| m.partial && m.attachments.iter().any(|a| a.inline && a.size > satchel_mime::KEEP_INLINE_BYTES as u64);
        let had_body = m.blob_hash.is_some();
        let mut raw = if big_inline(&m) { self.full_raw_for(id).await } else { self.raw_for(id).await };
        if !had_body && let Ok(Some(r)) = &raw {
            // Just downloaded, perhaps without attachment contents.
            m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
            let missing = |a: &Attachment| satchel_mime::attachment_bytes(r, &a.id).is_none_or(|(_, _, b)| b.is_empty());
            if big_inline(&m) && m.attachments.iter().any(|a| a.inline && a.size > satchel_mime::KEEP_INLINE_BYTES as u64 && missing(a)) {
                raw = self.full_raw_for(id).await;
            }
        }
        let raw = match raw {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("body fetch failed: {e:#}");
                None
            }
        };
        let settings = self.settings();
        let sender = m.summary.from.as_ref().map(|a| a.email.to_lowercase()).unwrap_or_default();
        let allow_remote = force_remote || settings.remote_image_senders.iter().any(|s| s == &sender);
        let mut invite = None;
        let mut warnings = vec![];
        let mut unsubscribe = None;
        let (text, html, blocked, attachments) = match &raw {
            Some(r) => {
                let p = satchel_mime::parse(r).unwrap_or_default();
                invite = self.invite_from(&m.summary.account_id, &p.calendar_parts);
                warnings = self.message_warnings(&m, &p, invite.as_ref());
                unsubscribe = p.unsubscribe.clone();
                let (html, blocked) = match &p.html {
                    Some(h) => {
                        let inline = satchel_mime::inline_data_urls(r);
                        let s = satchel_mime::sanitize_html(h, allow_remote, &inline);
                        (Some(s.html), s.had_remote_content && !allow_remote)
                    }
                    None => (None, false),
                };
                // A stored copy without attachment contents parses with zero sizes.
                (p.text, html, blocked, if m.partial && !m.attachments.is_empty() { m.attachments.clone() } else { p.attachments })
            }
            None => (m.summary.snippet.clone(), None, false, m.attachments.clone()),
        };
        Ok(MessageDetail {
            summary: m.summary,
            cc: m.cc,
            bcc: m.bcc,
            reply_to: m.reply_to,
            message_id_header: m.message_id_hdr,
            in_reply_to: m.in_reply_to,
            references: m.references,
            text_body: text,
            html_body: html,
            remote_images_blocked: blocked,
            remote_images_allowed: allow_remote,
            attachments,
            body_pending: raw.is_none(),
            invite,
            warnings,
            unsubscribe,
            sender_verdict: self.store.sender_rule(&sender).ok().flatten(),
        })
    }

    pub(crate) fn list_mailboxes(&self, caller: &Caller, account_id: Option<String>) -> Result<Vec<Mailbox>> {
        if let Some(a) = &account_id {
            self.check_account(caller, a)?;
        }
        let allowed = self.allowed_accounts(caller);
        Ok(self.store.list_mailboxes(account_id.as_deref())?.into_iter().filter(|m| allowed.as_ref().is_none_or(|ids| ids.contains(&m.account_id))).collect())
    }

    pub(crate) async fn get_thread(&self, caller: &Caller, thread_id: String) -> Result<Thread> {
        let t = self.store.get_thread_summary(&thread_id)?.ok_or_else(|| not_found("thread"))?;
        self.check_account(caller, &t.account_id)?;
        let ids = self.store.thread_message_ids(&thread_id)?;
        let cutoff = self.cutoff(caller);
        let mut messages = vec![];
        let mut seen = HashSet::new();
        for id in ids {
            let Some(row) = self.store.message_row(&id)? else { continue };
            if cutoff.is_some_and(|c| row.summary.date < c) {
                continue;
            }
            // Collapse copies of the same message stored in several folders.
            if let Some(h) = &row.message_id_hdr
                && !seen.insert(h.clone()) {
                    continue;
                }
            messages.push(self.message_detail(&id, false).await?);
        }
        Ok(Thread { thread_id, account_id: t.account_id, subject: t.subject, messages })
    }

    /// An attachment's content. Agents get text content; binary only when
    /// their policy allows it.
    pub(crate) async fn get_attachment(&self, caller: &Caller, message_id: &str, attachment_id: &str) -> Result<Value> {
        let raw = self.full_raw_for(message_id).await?.ok_or_else(|| not_found("message body"))?;
        let (filename, mime_type, data) = satchel_mime::attachment_bytes(&raw, attachment_id).ok_or_else(|| not_found("attachment"))?;
        if let Caller::Agent(a) = caller {
            let is_text = mime_type.starts_with("text/") || mime_type == "application/json" || mime_type.ends_with("+xml");
            if is_text {
                return Ok(json!({ "filename": filename, "mime_type": mime_type, "text": String::from_utf8_lossy(&data) }));
            }
            if !a.policy.allow_attachments {
                return Err(forbidden("binary attachments are not shared with this agent"));
            }
        }
        Ok(json!({ "filename": filename, "mime_type": mime_type, "data_base64": base64::engine::general_purpose::STANDARD.encode(&data) }))
    }

    pub(crate) async fn load_remote_images(&self, message_id: &str, always_for_sender: bool) -> Result<MessageDetail> {
        if always_for_sender
            && let Some(m) = self.store.message_row(message_id)?
                && let Some(f) = m.summary.from {
                    let mut s = self.settings();
                    let e = f.email.to_lowercase();
                    if !s.remote_image_senders.contains(&e) {
                        s.remote_image_senders.push(e);
                        self.save_settings(s)?;
                    }
                }
        self.message_detail(message_id, true).await
    }

    /// The person view: everything exchanged with one person, given any of
    /// their addresses.
    pub(crate) fn get_person(&self, emails: &[String]) -> Result<PersonSummary> {
        let (contact, emails) = self.person_addresses(emails)?;
        let mut p = self.store.person_summary(&emails)?;
        if let Some(c) = contact {
            if !c.fields.name.trim().is_empty() {
                p.name = Some(c.fields.name.trim().to_string());
            }
            p.contact_id = Some(c.id);
        }
        let f = ThreadFilter { with: p.addresses.iter().map(|a| a.email.clone()).collect(), exclude_trash_junk: true, ..Default::default() };
        if !f.with.is_empty() {
            p.threads = self.store.count_threads(&f)?;
        }
        Ok(p)
    }
}
