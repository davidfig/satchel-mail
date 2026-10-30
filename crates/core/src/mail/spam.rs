//! Spam handling: per-sender rules that outrank the server's filter,
//! phishing warnings, and unsubscribing. See docs/decisions/0024-spam-handling.md.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine as _;
use chrono::Utc;
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::{invalid, not_found};
use crate::store::{MessageRef, StoredMessage};

/// How long the correspondents snapshot is reused before it is rebuilt.
const CORRESPONDENTS_TTL: Duration = Duration::from_secs(60);

/// People the user writes to, for telling a friend from someone borrowing their name.
#[derive(Default)]
pub(crate) struct Correspondents {
    emails: HashSet<String>,
    /// Normalized display name -> addresses seen with it.
    names: HashMap<String, HashSet<String>>,
}

impl Correspondents {
    fn add(&mut self, email: &str, name: Option<&str>) {
        let email = email.trim().to_lowercase();
        if email.is_empty() {
            return;
        }
        if let Some(n) = name.map(norm_name).filter(|n| n.chars().count() >= 3) {
            self.names.entry(n).or_default().insert(email.clone());
        }
        self.emails.insert(email);
    }
}

/// Whether a parsed message doesn't fail the receiving server's SPF/DKIM/DMARC
/// checks. Weaker than `sender_verified`: no results at all also pass, and
/// any server's header counts, since a failure never grants trust.
pub(crate) fn not_forged(p: &satchel_mime::Parsed) -> bool {
    p.auth_results.as_deref().and_then(satchel_mime::auth_failure).is_none()
}

fn norm_name(n: &str) -> String {
    n.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace()).split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl Engine {
    // -- rules ------------------------------------------------------------------

    /// Senders of these messages, excluding the user's own addresses.
    fn senders_of(&self, ids: &[String]) -> Result<Vec<String>> {
        let own: HashSet<String> = self.store.list_accounts()?.into_iter().map(|a| a.email.to_lowercase()).collect();
        let mut out: Vec<String> = vec![];
        for id in ids {
            let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
            if let Some(f) = m.summary.from {
                let e = f.email.trim().to_lowercase();
                if !own.contains(&e) && !out.contains(&e) {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn mark_spam(&self, ids: &[String], actor: &str) -> Result<Value> {
        let senders = self.senders_of(ids)?;
        for s in &senders {
            self.store.set_sender_rule(s, Some(SenderVerdict::Spam))?;
        }
        for (account, refs) in self.group_by_account(ids)? {
            let junk = self.store.mailbox_by_role(&account, MailboxRole::Junk)?.ok_or_else(|| invalid("this account has no Spam folder"))?;
            let ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();
            self.move_messages(&ids, &junk.id, actor)?;
        }
        let moved = self.enforce_all_sender_rules()?;
        let mut invites_removed = 0;
        for (account, refs) in self.group_by_account(ids)? {
            let ids: Vec<String> = refs.into_iter().map(|r| r.id).collect();
            invites_removed += self.remove_spam_invites(&account, &ids)?;
        }
        self.store.audit(actor, "marked_spam", &json!({ "senders": senders, "messages": ids.len(), "invites_removed": invites_removed }))?;
        Ok(json!({ "senders": senders, "also_moved": moved, "invites_removed": invites_removed }))
    }

    pub(crate) fn mark_not_spam(&self, ids: &[String], actor: &str) -> Result<Value> {
        let senders = self.senders_of(ids)?;
        for s in &senders {
            self.store.set_sender_rule(s, Some(SenderVerdict::NotSpam))?;
        }
        for (account, refs) in self.group_by_account(ids)? {
            self.to_inbox(&account, refs, actor)?;
        }
        let moved = self.enforce_all_sender_rules()?;
        self.store.audit(actor, "marked_not_spam", &json!({ "senders": senders, "messages": ids.len() }))?;
        Ok(json!({ "senders": senders, "also_moved": moved }))
    }

    pub(crate) fn set_sender_rule(&self, email: &str, verdict: Option<SenderVerdict>) -> Result<()> {
        let email = email.trim().to_lowercase();
        if !email.contains('@') || email.contains(char::is_whitespace) {
            return Err(invalid(format!("invalid address: {email}")));
        }
        self.store.set_sender_rule(&email, verdict)?;
        self.store.audit("user", "sender_rule", &json!({ "email": email, "verdict": verdict }))?;
        self.enforce_all_sender_rules()?;
        Ok(())
    }

    /// Out of Junk and into the inbox. Label accounts drop the Spam label;
    /// folder accounts move.
    fn to_inbox(&self, account: &str, refs: Vec<MessageRef>, actor: &str) -> Result<()> {
        let inbox = self.store.mailbox_by_role(account, MailboxRole::Inbox)?.ok_or_else(|| not_found("inbox"))?;
        if self.uses_labels(account) {
            let junk: Vec<String> = self.store.mailbox_by_role(account, MailboxRole::Junk)?.into_iter().map(|m| m.id).collect();
            return self.labels_inner(account, refs, std::slice::from_ref(&inbox.id), &junk, actor);
        }
        let ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();
        self.move_messages(&ids, &inbox.id, actor)
    }

    fn enforce_all_sender_rules(&self) -> Result<usize> {
        let mut n = 0;
        for a in self.store.list_accounts()? {
            n += self.enforce_sender_rules(&a.id)?;
        }
        Ok(n)
    }

    /// Put messages where the user's sender rules say they belong, overriding
    /// the server's spam filter. Runs after every sync. Returns messages moved.
    pub(crate) fn enforce_sender_rules(&self, account_id: &str) -> Result<usize> {
        let misplaced = self.store.misplaced_by_rules(account_id)?;
        if misplaced.is_empty() {
            return Ok(0);
        }
        let (spam, ham): (Vec<_>, Vec<_>) = misplaced.into_iter().partition(|(_, v)| *v == SenderVerdict::Spam);
        let spam: Vec<String> = spam.into_iter().map(|(id, _)| id).collect();
        // A "not spam" rule trusts an address, and anyone can put any address
        // in From. Only mail whose From domain the server verified comes out
        // of Junk, so a spoofed "bank" stays there. Without a body to check
        // yet, it waits for the next sync.
        let trusted = if ham.is_empty() { Default::default() } else { self.auth_servers(account_id) };
        let ham: Vec<String> = ham.into_iter().map(|(id, _)| id).filter(|id| self.authentic(id, &trusted)).collect();
        if !spam.is_empty()
            && let Some(junk) = self.store.mailbox_by_role(account_id, MailboxRole::Junk)?
        {
            self.move_messages(&spam, &junk.id, "sync")?;
        }
        if !ham.is_empty() {
            let refs: Vec<MessageRef> = ham.iter().filter_map(|id| self.store.message_ref(id).ok().flatten()).collect();
            self.to_inbox(account_id, refs, "sync")?;
        }
        tracing::debug!(account = account_id, spam = spam.len(), not_spam = ham.len(), "sender rules moved messages");
        Ok(spam.len() + ham.len())
    }

    /// Whether a trusted server verified a downloaded message's From
    /// domain (see `sender_verified_by`).
    fn authentic(&self, id: &str, trusted: &satchel_mime::AuthServers) -> bool {
        let Ok(Some(m)) = self.store.message_row(id) else { return false };
        let Some(raw) = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten()) else { return false };
        satchel_mime::parse(&raw).is_some_and(|p| crate::mail::trust::sender_verified_by(&p, trusted))
    }

    /// Take events that spam invitations put on the calendar back off it,
    /// without telling the organizer: any reply, even a decline, confirms the
    /// address. Only events the user never answered and doesn't organize are
    /// removed. Returns events removed.
    ///
    /// An event is removed only if the message's sender organizes it, both in
    /// the invitation and on the calendar, and the message doesn't fail
    /// sender checks. Matching on UID alone would let
    /// anyone who learns a real meeting's UID get it deleted by sending a
    /// spoofed invitation and having it marked as spam; and shared senders
    /// (calendar notification services, mailing lists) relay invitations
    /// from people the user does want to hear from.
    pub(crate) fn remove_spam_invites(&self, account_id: &str, message_ids: &[String]) -> Result<usize> {
        let Some(acc) = self.store.get_account(account_id)? else { return Ok(0) };
        let mut removed = 0;
        for id in message_ids {
            let Some(m) = self.store.message_row(id)? else { continue };
            let sender = m.summary.from.as_ref().map(|a| a.email.trim().to_lowercase()).unwrap_or_default();
            let Some(raw) = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten()) else { continue };
            let Some(p) = satchel_mime::parse(&raw) else { continue };
            // A message failing sender checks may not be from its organizer.
            if !not_forged(&p) {
                continue;
            }
            // (UID, organizer) of each invited event the sender organizes.
            let invited: HashSet<(String, String)> = p
                .calendar_parts
                .iter()
                .flat_map(|ics| satchel_ical::parse(ics).events)
                .filter_map(|e| Some((e.uid, e.organizer?.email.trim().to_lowercase())))
                .filter(|(_, org)| !sender.is_empty() && *org == sender)
                .collect();
            for (uid, _) in invited {
                for stored in self.store.events_by_uid(account_id, &uid)? {
                    let Some(ev) = crate::calendar::get_event(&self.store, &stored.id)? else { continue };
                    let organizer_is_me = ev.organizer.as_ref().is_some_and(|o| o.email.eq_ignore_ascii_case(&acc.email));
                    let answered = !matches!(ev.my_status, None | Some(PartStat::NeedsAction));
                    let Some(cal) = self.store.get_calendar(&stored.calendar_id)? else { continue };
                    let sender_organizes = ev.organizer.as_ref().is_some_and(|o| o.email.trim().eq_ignore_ascii_case(&sender));
                    if organizer_is_me || answered || cal.cal.read_only || !sender_organizes {
                        continue;
                    }
                    if !self.store.delete_event(&stored.id)? {
                        continue;
                    }
                    self.enqueue(
                        account_id,
                        "cal_delete",
                        json!({ "calendar_remote_id": cal.remote_id, "remote_id": stored.remote_id, "etag": stored.etag, "notify": false }),
                        "sync",
                    )?;
                    self.store.audit("sync", "spam_invite_removed", &json!({ "title": ev.title, "organizer": ev.organizer.map(|o| o.email) }))?;
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            self.emit(CoreEvent::CalendarChanged { account_id: account_id.to_string() });
        }
        Ok(removed)
    }

    /// After a calendar sync: invitations from blocked senders that the
    /// server added since the message was marked come back off. Unlike
    /// marking one message, this runs unasked, so only for events the
    /// blocked sender organizes.
    pub(crate) fn remove_blocked_invites(&self, account_id: &str) -> Result<usize> {
        let since = Utc::now() - chrono::Duration::days(60);
        let ids = self.store.spam_rule_messages_with_attachments(account_id, since)?;
        self.remove_spam_invites(account_id, &ids)
    }

    // -- warnings -----------------------------------------------------------------

    fn correspondents(&self) -> Arc<Correspondents> {
        let mut cache = self.correspondents.lock().unwrap();
        if let Some((at, c)) = cache.as_ref()
            && at.elapsed() < CORRESPONDENTS_TTL
        {
            return c.clone();
        }
        let mut c = Correspondents::default();
        for (email, name) in self.store.correspondents().unwrap_or_default() {
            c.add(&email, name.as_deref());
        }
        // Someone using your own name from another address is the classic CEO scam.
        for a in self.store.list_accounts().unwrap_or_default() {
            c.add(&a.email, Some(&a.display_name));
        }
        let c = Arc::new(c);
        *cache = Some((Instant::now(), c.clone()));
        c
    }

    fn is_known(&self, email: &str) -> bool {
        let e = email.trim().to_lowercase();
        self.correspondents().emails.contains(&e) || self.store.sender_rule(&e).ok().flatten() == Some(SenderVerdict::NotSpam)
    }

    /// Phishing and safety signals for one message, worst first.
    pub(crate) fn message_warnings(&self, m: &StoredMessage, p: &satchel_mime::Parsed, invite: Option<&Invite>) -> Vec<MessageWarning> {
        let mut out = vec![];
        let warn = |out: &mut Vec<MessageWarning>, kind, message: String| out.push(MessageWarning { kind, message });
        let from = m.summary.from.as_ref();
        let from_email = from.map(|f| f.email.to_lowercase()).unwrap_or_default();
        let from_domain = satchel_mime::email_domain(&from_email);
        let known = !from_email.is_empty() && self.is_known(&from_email);

        if let Some(f) = from
            && !known
        {
            let name = f.name.as_deref().unwrap_or("");
            let lookalike_addr = name.contains('@') && !name.to_lowercase().contains(&from_email);
            let borrowed = self.correspondents().names.get(&norm_name(name)).filter(|emails| !emails.contains(&from_email)).and_then(|e| e.iter().next().cloned());
            if lookalike_addr {
                warn(&mut out, WarningKind::Impersonation, format!("The name shows an email address, but this was sent from {from_email}."));
            } else if let Some(real) = borrowed {
                warn(&mut out, WarningKind::Impersonation, format!("\"{}\" usually writes from {real}, but this came from {from_email}.", name.trim()));
            }
        }
        if let Some(why) = p.auth_results.as_deref().and_then(satchel_mime::auth_failure) {
            let domain = if from_domain.is_empty() { "the sender".to_string() } else { from_domain.clone() };
            warn(&mut out, WarningKind::AuthFailed, format!("This message {why}, so it may not really be from {domain}."));
        }
        if let Some(h) = &p.html {
            for (shown, actual) in satchel_mime::deceptive_links(h).into_iter().take(2) {
                warn(&mut out, WarningKind::DeceptiveLink, format!("A link shows {shown} but goes to {actual}."));
            }
        }
        if let Some(rt) = m.reply_to.first() {
            let rt_email = rt.email.to_lowercase();
            let rt_domain = satchel_mime::email_domain(&rt_email);
            if !from_domain.is_empty() && satchel_mime::base_domain(&rt_domain) != satchel_mime::base_domain(&from_domain) && !self.is_known(&rt_email) {
                warn(&mut out, WarningKind::ReplyTo, format!("Replies go to {rt_email}, not to {from_email}."));
            }
        }
        for a in p.attachments.iter().filter(|a| a.risky) {
            let what = if satchel_mime::is_archive(&a.filename) { "is an archive that can hide programs" } else { "can run code or imitate a sign-in page" };
            warn(&mut out, WarningKind::RiskyAttachment, format!("{} {what}. Don't open it unless you expected it.", a.filename));
        }
        if let Some(inv) = invite
            && let Some(org) = &inv.organizer
            && !self.is_known(&org.email)
        {
            warn(&mut out, WarningKind::UnknownInviter, format!("You haven't written to {} before. Responding, even to decline, tells them your address works.", org.email));
        }
        out
    }

    // -- unsubscribe ------------------------------------------------------------------

    pub(crate) async fn unsubscribe(&self, message_id: &str) -> Result<Value> {
        let m = self.store.message_row(message_id)?.ok_or_else(|| not_found("message"))?;
        let raw = self.raw_for(message_id).await?.ok_or_else(|| not_found("message body"))?;
        let parsed = satchel_mime::parse(&raw).unwrap_or_default();
        let u = parsed.unsubscribe.ok_or_else(|| invalid("this message has no unsubscribe link"))?;
        let sender = m.summary.from.as_ref().map(|f| f.email.clone()).unwrap_or_default();
        if u.one_click
            && let Some(url) = &u.url
        {
            self.factory.one_click_unsubscribe(url).await?;
            self.store.audit("user", "unsubscribed", &json!({ "sender": sender, "via": "one_click" }))?;
            return Ok(json!({ "via": "one_click" }));
        }
        // Sends exactly what the confirmation showed (`Unsubscribe::email`):
        // one validated address, one-line subject and body.
        if let Some(e) = u.email {
            let acc = self.store.get_account(&m.summary.account_id)?.ok_or_else(|| not_found("account"))?;
            let to = e.to.clone();
            let o = satchel_mime::Outgoing {
                from: Address { name: Some(acc.display_name.clone()).filter(|s| !s.is_empty()), email: acc.email.clone() },
                to: vec![Address::new(to.clone())],
                subject: e.subject,
                text: e.body,
                message_id: satchel_mime::new_message_id(&satchel_mime::email_domain(&acc.email)),
                date: Utc::now(),
                ..Default::default()
            };
            let raw = satchel_mime::build(&o);
            self.enqueue(
                &acc.id,
                "send",
                json!({ "raw": base64::engine::general_purpose::STANDARD.encode(&raw), "rcpts": [to], "from": acc.email, "subject": o.subject }),
                "user",
            )?;
            self.store.audit("user", "unsubscribed", &json!({ "sender": sender, "via": "email", "to": to }))?;
            return Ok(json!({ "via": "email", "to": to }));
        }
        let url = u.url.ok_or_else(|| invalid("this message has no usable unsubscribe link"))?;
        Ok(json!({ "via": "browser", "url": url }))
    }
}
