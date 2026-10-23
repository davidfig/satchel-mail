//! Command dispatch. Every client request lands in [`Engine::handle`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use base64::Engine as _;
use chrono::{Duration, Utc};
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::{Caller, Engine, Mode};
use crate::error::{err, forbidden, invalid, not_found, to_api};
use crate::provider::CredentialHandle;
use crate::secrets::{MemSecretStore, SecretStore, account_key};
use crate::store::{MessageRef, ThreadFilter, new_id};

fn ok<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

impl Engine {
    pub async fn handle(self: &Arc<Self>, caller: &Caller, cmd: Command) -> std::result::Result<Value, ApiError> {
        if let Caller::Agent(a) = caller {
            if a.revoked {
                return Err(ApiError::new(ErrorCode::Auth, "agent revoked"));
            }
            if !cmd.allowed_for_agent() {
                return Err(ApiError::new(ErrorCode::Forbidden, "this action requires the user; agents can only read and propose"));
            }
        }
        self.dispatch(caller, cmd).await.map_err(to_api)
    }

    // -- agent policy helpers ------------------------------------------------

    fn allowed_accounts(&self, caller: &Caller) -> Option<Vec<String>> {
        match caller {
            Caller::User => None,
            Caller::Agent(a) => a.policy.account_ids.clone(),
        }
    }

    fn check_account(&self, caller: &Caller, account_id: &str) -> Result<()> {
        if let Some(ids) = self.allowed_accounts(caller)
            && !ids.iter().any(|x| x == account_id) {
                return Err(forbidden("account not shared with this agent"));
            }
        Ok(())
    }

    fn cutoff(&self, caller: &Caller) -> Option<chrono::DateTime<Utc>> {
        match caller {
            Caller::Agent(a) => a.policy.since_days.map(|d| Utc::now() - Duration::days(d as i64)),
            Caller::User => None,
        }
    }

    fn check_calendar_access(&self, caller: &Caller) -> Result<()> {
        if let Caller::Agent(a) = caller
            && !a.policy.allow_calendar {
                return Err(forbidden("calendar access is not enabled for this agent"));
            }
        Ok(())
    }

    fn check_message(&self, caller: &Caller, id: &str) -> Result<crate::store::StoredMessage> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        self.check_account(caller, &m.summary.account_id)?;
        if let Some(c) = self.cutoff(caller)
            && m.summary.date < c {
                return Err(forbidden("message is outside this agent's allowed date range"));
            }
        Ok(m)
    }

    fn require_user(&self, caller: &Caller) -> Result<()> {
        match caller {
            Caller::User => Ok(()),
            Caller::Agent(_) => Err(forbidden("requires the user")),
        }
    }

    fn require_full(&self) -> Result<()> {
        if self.mode != Mode::Full {
            return Err(err(ErrorCode::Conflict, "this process is read-only; open Satchel Mail to make changes"));
        }
        Ok(())
    }

    // -- dispatch -------------------------------------------------------------

    async fn dispatch(self: &Arc<Self>, caller: &Caller, cmd: Command) -> Result<Value> {
        match cmd {
            Command::Info => ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "mode": if self.mode == Mode::Full { "full" } else { "agent" },
                "data_dir": self.data_dir,
                "mirror_root": crate::mirror::root(&self.settings().mirror),
                "caller": caller.actor(),
            })),

            // ---------------------------------------------------------- accounts
            Command::ListAccounts => {
                let allowed = self.allowed_accounts(caller);
                let accounts: Vec<Account> = self
                    .store
                    .list_accounts()?
                    .into_iter()
                    .filter(|a| allowed.as_ref().is_none_or(|ids| ids.contains(&a.id)))
                    .collect();
                ok(accounts)
            }
            Command::TestAccount { config, credential } => {
                self.require_user(caller)?;
                self.test_account(&config, &credential).await?;
                ok(json!({ "ok": true }))
            }
            Command::AddAccount { config, credential } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.add_account(config, credential).await?)
            }
            Command::RemoveAccount { account_id } => {
                self.require_user(caller)?;
                self.stop_account(&account_id);
                let acc = self.store.get_account(&account_id)?.ok_or_else(|| not_found("account"))?;
                self.store.delete_account(&account_id)?;
                let _ = self.secrets.delete(&account_key(&account_id));
                self.store.audit("user", "account_removed", &json!({ "email": acc.email }))?;
                self.mirror_wake.notify_one();
                self.emit(CoreEvent::AccountsChanged);
                ok(json!({ "ok": true }))
            }
            Command::UpdateAccount { account_id, config, credential } => {
                self.require_user(caller)?;
                self.require_full()?;
                let acc = self.store.get_account(&account_id)?.ok_or_else(|| not_found("account"))?;
                if acc.kind != config.kind() {
                    return Err(invalid("account type can't change; add a new account instead"));
                }
                let cred = match credential {
                    Some(c) => c,
                    None => self.secrets.get(&account_key(&account_id))?.ok_or_else(|| err(ErrorCode::Auth, "no stored credential"))?,
                };
                self.test_account(&config, &cred).await?;
                self.stop_account(&account_id);
                self.store.update_account_config(&account_id, &config)?;
                self.secrets.set(&account_key(&account_id), &cred)?;
                let acc = self.store.get_account(&account_id)?.ok_or_else(|| not_found("account"))?;
                self.start_account(&acc);
                self.store.audit("user", "account_updated", &json!({ "email": acc.email }))?;
                self.emit(CoreEvent::AccountsChanged);
                ok(acc)
            }
            Command::StartOAuth { provider } => {
                self.require_user(caller)?;
                self.require_full()?;
                let weak = Arc::downgrade(self);
                let done: crate::provider::OAuthDone = Box::new(move |res| {
                    let Some(engine) = weak.upgrade() else { return };
                    tokio::spawn(async move {
                        let r = match res {
                            Ok((config, credential)) => engine.add_account(config, credential).await.map(|_| ()),
                            Err(e) => Err(e),
                        };
                        if let Err(e) = r {
                            engine.emit(CoreEvent::Notice { level: "error".into(), message: format!("Sign-in failed: {}", to_api(e).message) });
                        }
                    });
                });
                ok(json!({ "auth_url": self.factory.oauth_start(provider, done)? }))
            }
            Command::SyncNow { account_id } => {
                self.require_user(caller)?;
                match account_id {
                    Some(a) => self.wake_account(&a),
                    None => {
                        for a in self.store.list_accounts()? {
                            self.wake_account(&a.id);
                        }
                    }
                }
                ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------------- mail reads
            Command::ListMailboxes { account_id } => {
                if let Some(a) = &account_id {
                    self.check_account(caller, a)?;
                }
                let allowed = self.allowed_accounts(caller);
                let mbs: Vec<Mailbox> = self
                    .store
                    .list_mailboxes(account_id.as_deref())?
                    .into_iter()
                    .filter(|m| allowed.as_ref().is_none_or(|ids| ids.contains(&m.account_id)))
                    .collect();
                ok(mbs)
            }
            Command::ListThreads { query } => ok(self.list_threads(caller, &query)?),
            Command::CountThreads { query } => {
                let f = self.thread_filter(caller, &query)?;
                ok(json!({ "total": self.store.count_threads(&f)? }))
            }
            Command::GetThread { thread_id } => {
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
                ok(Thread { thread_id, account_id: t.account_id, subject: t.subject, messages })
            }
            Command::GetMessage { message_id } => {
                self.check_message(caller, &message_id)?;
                ok(self.message_detail(&message_id, false).await?)
            }
            Command::GetRawMessage { message_id } => {
                self.require_user(caller)?;
                let raw = self.raw_for(&message_id).await?.ok_or_else(|| not_found("message body"))?;
                ok(json!({ "raw": String::from_utf8_lossy(&raw) }))
            }
            Command::GetAttachment { message_id, attachment_id } => {
                self.check_message(caller, &message_id)?;
                let raw = self.raw_for(&message_id).await?.ok_or_else(|| not_found("message body"))?;
                let (filename, mime_type, data) = satchel_mime::attachment_bytes(&raw, &attachment_id).ok_or_else(|| not_found("attachment"))?;
                if let Caller::Agent(a) = caller {
                    // Agents get text content; binary only when explicitly allowed.
                    let is_text = mime_type.starts_with("text/") || mime_type == "application/json" || mime_type.ends_with("+xml");
                    if is_text {
                        return ok(json!({ "filename": filename, "mime_type": mime_type, "text": String::from_utf8_lossy(&data) }));
                    }
                    if !a.policy.allow_attachments {
                        return Err(forbidden("binary attachments are not shared with this agent"));
                    }
                }
                ok(json!({ "filename": filename, "mime_type": mime_type, "data_base64": base64::engine::general_purpose::STANDARD.encode(&data) }))
            }
            Command::LoadRemoteImages { message_id, always_for_sender } => {
                self.require_user(caller)?;
                if always_for_sender
                    && let Some(m) = self.store.message_row(&message_id)?
                        && let Some(f) = m.summary.from {
                            let mut s = self.settings();
                            let e = f.email.to_lowercase();
                            if !s.remote_image_senders.contains(&e) {
                                s.remote_image_senders.push(e);
                                self.save_settings(s)?;
                            }
                        }
                ok(self.message_detail(&message_id, true).await?)
            }

            // ---------------------------------------------------------- mail writes
            Command::SetFlags { message_ids, seen, flagged } => {
                self.require_full()?;
                self.set_flags(&message_ids, seen, flagged, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::Move { message_ids, mailbox_id } => {
                self.require_full()?;
                self.move_messages(&message_ids, &mailbox_id, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::Archive { message_ids } => {
                self.require_full()?;
                self.archive(&message_ids, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::Trash { message_ids } => {
                self.require_full()?;
                self.trash(&message_ids, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::SetLabels { message_ids, add, remove } => {
                self.require_full()?;
                self.set_labels(&message_ids, &add, &remove, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::SaveDraft { draft } => {
                self.require_full()?;
                ok(json!({ "draft_id": self.save_draft(draft, true)? }))
            }
            Command::DeleteDraft { draft_id } => {
                self.require_full()?;
                self.delete_draft(&draft_id)?;
                ok(json!({ "ok": true }))
            }
            Command::ListDrafts => ok(self.store.list_drafts()?),
            Command::Send { draft } => {
                self.require_full()?;
                ok(json!({ "op_id": self.send(draft, &caller.actor())? }))
            }
            Command::ComposeReply { message_id, mode } => ok(self.compose_reply(&message_id, mode).await?),
            Command::SuggestAddresses { prefix } => {
                self.require_user(caller)?;
                ok(self.store.known_senders(prefix.trim(), 8)?)
            }

            // ---------------------------------------------------------- calendar
            Command::ListCalendars => {
                self.check_calendar_access(caller)?;
                let allowed = self.allowed_accounts(caller);
                let cals: Vec<Calendar> = self
                    .store
                    .list_calendars(None)?
                    .into_iter()
                    .map(|c| c.cal)
                    .filter(|c| allowed.as_ref().is_none_or(|ids| ids.contains(&c.account_id)))
                    .collect();
                ok(cals)
            }
            Command::SetCalendarVisible { calendar_id, visible } => {
                self.store.set_calendar_visible(&calendar_id, visible)?;
                self.emit(CoreEvent::CalendarChanged { account_id: String::new() });
                ok(json!({ "ok": true }))
            }
            Command::ListEvents { from, to, calendar_ids } => {
                self.check_calendar_access(caller)?;
                if to - from > Duration::days(400) {
                    return Err(invalid("range too large (max 400 days)"));
                }
                let ids = self.visible_calendar_ids(caller, calendar_ids)?;
                ok(crate::calendar::occurrences(&self.store, from, to, Some(&ids))?)
            }
            Command::GetEvent { event_id } => {
                self.check_calendar_access(caller)?;
                let e = crate::calendar::get_event(&self.store, &event_id)?.ok_or_else(|| not_found("event"))?;
                let cal = self.store.get_calendar(&e.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
                self.check_account(caller, &cal.cal.account_id)?;
                ok(e)
            }
            Command::CreateEvent { event } => {
                self.require_full()?;
                ok(self.create_event(event, &caller.actor())?)
            }
            Command::UpdateEvent { event } => {
                self.require_full()?;
                ok(self.update_event(event, &caller.actor())?)
            }
            Command::DeleteEvent { event_id } => {
                self.require_full()?;
                self.delete_event(&event_id, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::RespondToInvite { event_id, response } => {
                self.require_full()?;
                self.respond(&event_id, response, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::RespondToEmailInvite { message_id, response } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.respond_email_invite(&message_id, response).await?)
            }
            Command::FindFreeTime { from, to, duration_minutes, day_start_hour, day_end_hour, timezone } => {
                self.check_calendar_access(caller)?;
                let tz: chrono_tz::Tz = timezone.as_deref().unwrap_or("UTC").parse().map_err(|_| invalid("unknown timezone"))?;
                let ids = self.visible_calendar_ids(caller, None)?;
                let busy = crate::calendar::occurrences(&self.store, from, to, Some(&ids))?;
                ok(crate::calendar::free_slots(
                    &busy,
                    from,
                    to,
                    Duration::minutes(duration_minutes.max(5) as i64),
                    day_start_hour.unwrap_or(9),
                    day_end_hour.unwrap_or(17),
                    tz,
                ))
            }

            // ---------------------------------------------------------- proposals
            Command::ListProposals { status } => {
                let agent = match caller {
                    Caller::Agent(a) => Some(a.id.clone()),
                    Caller::User => None,
                };
                ok(self.store.list_proposals(status, agent.as_deref())?)
            }
            Command::GetProposal { proposal_id } => {
                let (p, _) = self.store.get_proposal(&proposal_id)?.ok_or_else(|| not_found("proposal"))?;
                if let Caller::Agent(a) = caller
                    && p.agent_id != a.id {
                        return Err(not_found("proposal"));
                    }
                ok(p)
            }
            Command::CreateProposal { action, rationale, source_thread_ids } => {
                let Caller::Agent(agent) = caller else {
                    return Err(invalid("proposals come from agents; act directly instead"));
                };
                ok(self.create_proposal(agent, action, rationale, source_thread_ids)?)
            }
            Command::ApproveProposal { proposal_id, edited } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.approve_proposal(&proposal_id, edited).await?)
            }
            Command::RejectProposal { proposal_id, note } => {
                self.require_user(caller)?;
                let (p, _) = self.store.get_proposal(&proposal_id)?.ok_or_else(|| not_found("proposal"))?;
                self.store.set_proposal_status(&proposal_id, ProposalStatus::Rejected, note.as_deref(), None)?;
                self.store.audit("user", "proposal_rejected", &json!({ "proposal": proposal_id, "agent": p.agent_name, "summary": p.summary, "note": note }))?;
                self.emit(CoreEvent::ProposalsChanged);
                ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------------- agents
            Command::ListAgents => {
                self.require_user(caller)?;
                ok(self.store.list_agents()?)
            }
            Command::CreateAgent { name, policy } => {
                self.require_user(caller)?;
                use rand::RngCore;
                let mut bytes = [0u8; 24];
                rand::rng().fill_bytes(&mut bytes);
                let token = format!("stk_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes));
                let agent = self.store.insert_agent(&name, &Self::hash_token(&token), &policy)?;
                self.store.audit("user", "agent_created", &json!({ "agent": name }))?;
                ok(json!({ "agent": agent, "token": token }))
            }
            Command::UpdateAgent { agent_id, name, policy } => {
                self.require_user(caller)?;
                self.store.update_agent(&agent_id, name.as_deref(), policy.as_ref())?;
                ok(self.store.get_agent(&agent_id)?)
            }
            Command::RevokeAgent { agent_id } => {
                self.require_user(caller)?;
                self.store.revoke_agent(&agent_id)?;
                self.store.audit("user", "agent_revoked", &json!({ "agent": agent_id }))?;
                ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------------- misc
            Command::GetSettings => ok(self.settings()),
            Command::UpdateSettings { settings } => {
                self.require_user(caller)?;
                self.save_settings(settings)?;
                ok(self.settings())
            }
            Command::ListAudit { limit } => {
                self.require_user(caller)?;
                ok(self.store.list_audit(limit.unwrap_or(200).min(2000))?)
            }
            Command::ListOutbox => ok(self.store.list_outbox()?),
            Command::RetryOutbox { id } => {
                self.require_user(caller)?;
                self.store.op_reset(&id)?;
                for a in self.store.list_accounts()? {
                    self.wake_account(&a.id);
                }
                ok(json!({ "ok": true }))
            }
        }
    }

    // -- settings ---------------------------------------------------------------

    pub(crate) fn save_settings(&self, s: Settings) -> Result<()> {
        let old = self.settings();
        self.store.set_setting("settings", &s)?;
        let mirror_changed = serde_json::to_string(&old.mirror)? != serde_json::to_string(&s.mirror)?;
        *self.settings.write().unwrap() = s.clone();
        if mirror_changed {
            if old.mirror.root != s.mirror.root || !s.mirror.enabled {
                let _ = std::fs::remove_dir_all(crate::mirror::root(&old.mirror).join("mail"));
            }
            self.store.reset_mirrored()?;
            self.mirror_wake.notify_one();
        }
        Ok(())
    }

    // -- accounts ---------------------------------------------------------------

    pub(crate) async fn add_account(self: &Arc<Self>, config: AccountConfig, credential: Credential) -> Result<Account> {
        let email = config.email().to_lowercase();
        if self.store.list_accounts()?.iter().any(|a| a.email.to_lowercase() == email && a.kind == config.kind()) {
            return Err(err(ErrorCode::Conflict, format!("{email} is already added")));
        }
        self.test_account(&config, &credential).await?;
        let acc = self.store.insert_account(&config)?;
        self.secrets.set(&account_key(&acc.id), &credential)?;
        self.store.audit("user", "account_added", &json!({ "email": acc.email, "kind": acc.kind }))?;
        self.start_account(&acc);
        self.emit(CoreEvent::AccountsChanged);
        Ok(acc)
    }

    async fn test_account(&self, config: &AccountConfig, credential: &Credential) -> Result<()> {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecretStore::default());
        let tmp = Account {
            id: "test".into(),
            kind: config.kind(),
            email: config.email().into(),
            display_name: config.display_name().into(),
            config: config.clone(),
            sync: SyncStatus::default(),
            color: String::new(),
        };
        secrets.set(&account_key("test"), credential)?;
        let p = self.factory.create(&tmp, CredentialHandle::new("test".into(), secrets))?;
        if let Some(m) = &p.mail {
            m.test().await?;
        }
        Ok(())
    }

    // -- mail reads ---------------------------------------------------------------

    fn list_threads(&self, caller: &Caller, q: &ThreadQuery) -> Result<Vec<ThreadSummary>> {
        let f = self.thread_filter(caller, q)?;
        self.store.list_threads(&f)
    }

    fn thread_filter(&self, caller: &Caller, q: &ThreadQuery) -> Result<ThreadFilter> {
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
            let p = crate::search::parse_query(s);
            f.from = p.from;
            f.to = p.to;
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
            if !p.text.trim().is_empty() {
                f.message_ids = Some(self.search.search(&p.text, 10_000)?);
            }
        }
        Ok(f)
    }

    async fn raw_for(&self, id: &str) -> Result<Option<Vec<u8>>> {
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

    pub(crate) async fn message_detail(&self, id: &str, force_remote: bool) -> Result<MessageDetail> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        let raw = match self.raw_for(id).await {
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
        let (text, html, blocked, attachments) = match &raw {
            Some(r) => {
                let p = satchel_mime::parse(r).unwrap_or_default();
                invite = self.invite_from(&m.summary.account_id, &p.calendar_parts);
                let (html, blocked) = match &p.html {
                    Some(h) => {
                        let inline = satchel_mime::inline_data_urls(r);
                        let s = satchel_mime::sanitize_html(h, allow_remote, &inline);
                        (Some(s.html), s.had_remote_content && !allow_remote)
                    }
                    None => (None, false),
                };
                (p.text, html, blocked, p.attachments)
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
            attachments,
            body_pending: raw.is_none(),
            invite,
        })
    }

    fn invite_from(&self, account_id: &str, parts: &[String]) -> Option<Invite> {
        let ics = parts.first()?;
        let cal = satchel_ical::parse(ics);
        let e = cal.events.first()?;
        let me = self.store.get_account(account_id).ok().flatten()?.email;
        let event_id = self.store.events_by_uid(account_id, &e.uid).ok().and_then(|v| v.into_iter().next()).map(|x| x.id);
        let my_status = match &event_id {
            Some(id) => crate::calendar::get_event(&self.store, id).ok().flatten().and_then(|ev| ev.my_status),
            None => e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(&me)).map(|a| a.status),
        };
        Some(Invite {
            uid: e.uid.clone(),
            method: cal.method.clone().unwrap_or_else(|| "PUBLISH".into()),
            title: e.summary.clone(),
            start: e.start_utc(),
            end: e.end_utc(),
            all_day: e.start.is_date(),
            location: e.location.clone(),
            organizer: e.organizer.clone(),
            my_status,
            event_id,
        })
    }

    /// RSVP to an emailed invitation. If the event is already in a synced
    /// calendar, respond there (the server notifies the organizer). Otherwise add
    /// it to the account's first writable calendar and email an iMIP REPLY.
    async fn respond_email_invite(&self, message_id: &str, response: PartStat) -> Result<Value> {
        let m = self.store.message_row(message_id)?.ok_or_else(|| not_found("message"))?;
        let acc = self.store.get_account(&m.summary.account_id)?.ok_or_else(|| not_found("account"))?;
        let raw = self.raw_for(message_id).await?.ok_or_else(|| not_found("message body"))?;
        let parsed = satchel_mime::parse(&raw).unwrap_or_default();
        let ics = parsed.calendar_parts.first().ok_or_else(|| invalid("this message has no invitation"))?;
        let cal = satchel_ical::parse(ics);
        let ev = cal.events.first().ok_or_else(|| invalid("invitation has no event"))?.clone();
        if let Some(existing) = self.store.events_by_uid(&acc.id, &ev.uid)?.into_iter().next() {
            self.respond(&existing.id, response, "user")?;
            return Ok(json!({ "ok": true, "via": "calendar", "event_id": existing.id }));
        }
        // Not synced: add a copy and reply by email.
        let target = self.store.list_calendars(Some(&acc.id))?.into_iter().find(|c| !c.cal.read_only);
        let mut event_id = None;
        if let Some(c) = target {
            let with_status = satchel_ical::set_partstat(ics, &acc.email, response);
            let body: String = {
                let p = satchel_ical::parse(&with_status);
                let range = crate::calendar::series_range(&p.events);
                // Strip METHOD: stored objects must not carry it.
                let clean: String = with_status.lines().filter(|l| !l.starts_with("METHOD:")).map(|l| format!("{}\n", l.trim_end_matches('\r'))).collect();
                let id = self.store.upsert_event(&c.cal.id, &acc.id, &format!("local:{}", new_id()), None, &ev.uid, &clean, &ev.summary, range, ev.rrule.is_some(), true)?;
                event_id = Some(id.clone());
                id
            };
            self.enqueue(&acc.id, "cal_put", json!({ "event_id": body }), "user")?;
            self.emit(CoreEvent::CalendarChanged { account_id: acc.id.clone() });
        }
        if let Some(org) = &ev.organizer {
            let reply = satchel_ical::reply(&ev, &acc.email, Some(&acc.display_name), response);
            let verb = match response {
                PartStat::Accepted => "Accepted",
                PartStat::Declined => "Declined",
                PartStat::Tentative => "Tentative",
                PartStat::NeedsAction => "Updated",
            };
            let o = satchel_mime::Outgoing {
                from: Address { name: Some(acc.display_name.clone()).filter(|s| !s.is_empty()), email: acc.email.clone() },
                to: vec![org.clone()],
                subject: format!("{verb}: {}", ev.summary),
                text: format!("{} has {} this invitation.", if acc.display_name.is_empty() { &acc.email } else { &acc.display_name }, verb.to_lowercase()),
                attachments: vec![("invite.ics".into(), "text/calendar; method=REPLY; charset=utf-8".into(), reply.into_bytes())],
                message_id: satchel_mime::new_message_id(acc.email.split('@').nth(1).unwrap_or("satchel.local")),
                date: Utc::now(),
                ..Default::default()
            };
            let raw = satchel_mime::build(&o);
            self.enqueue(
                &acc.id,
                "send",
                json!({ "raw": base64::engine::general_purpose::STANDARD.encode(&raw), "rcpts": [org.email], "from": acc.email, "subject": o.subject }),
                "user",
            )?;
        }
        self.store.audit("user", "invite_response", &json!({ "event": ev.summary, "response": satchel_ical::partstat_str(response), "via": "email" }))?;
        Ok(json!({ "ok": true, "via": "email", "event_id": event_id }))
    }

    async fn compose_reply(&self, message_id: &str, mode: ReplyMode) -> Result<Draft> {
        let m = self.store.message_row(message_id)?.ok_or_else(|| not_found("message"))?;
        let acc = self.store.get_account(&m.summary.account_id)?.ok_or_else(|| not_found("account"))?;
        let detail = self.message_detail(message_id, false).await?;
        let me = acc.email.to_lowercase();
        let not_me = |a: &Address| a.email.to_lowercase() != me;
        let from_me = m.summary.from.as_ref().is_some_and(|f| !not_me(f));
        let primary: Vec<Address> = if !m.reply_to.is_empty() {
            m.reply_to.clone()
        } else if from_me {
            m.summary.to.clone()
        } else {
            m.summary.from.clone().into_iter().collect()
        };
        let mut d = Draft { account_id: acc.id.clone(), ..Default::default() };
        match mode {
            ReplyMode::Reply => {
                d.to = primary;
                d.subject = satchel_mime::reply_subject(&m.summary.subject);
                d.body_text = satchel_mime::quote_for_reply(&detail.text_body, m.summary.from.as_ref(), Some(m.summary.date));
                d.in_reply_to = Some(message_id.to_string());
            }
            ReplyMode::ReplyAll => {
                let mut to = primary;
                for a in &m.summary.to {
                    if not_me(a) && !to.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
                        to.push(a.clone());
                    }
                }
                d.cc = m.cc.iter().filter(|a| not_me(a) && !to.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email))).cloned().collect();
                d.to = to;
                d.subject = satchel_mime::reply_subject(&m.summary.subject);
                d.body_text = satchel_mime::quote_for_reply(&detail.text_body, m.summary.from.as_ref(), Some(m.summary.date));
                d.in_reply_to = Some(message_id.to_string());
            }
            ReplyMode::Forward => {
                d.subject = satchel_mime::forward_subject(&m.summary.subject);
                let parsed = satchel_mime::Parsed {
                    subject: m.summary.subject.clone(),
                    from: m.summary.from.clone(),
                    to: m.summary.to.clone(),
                    date: Some(m.summary.date),
                    text: detail.text_body.clone(),
                    ..Default::default()
                };
                d.body_text = satchel_mime::forward_body(&parsed);
                d.forward_of = Some(message_id.to_string());
            }
        }
        Ok(d)
    }

    // -- mail writes ---------------------------------------------------------------

    fn group_by_account(&self, ids: &[String]) -> Result<HashMap<String, Vec<MessageRef>>> {
        let mut out: HashMap<String, Vec<MessageRef>> = HashMap::new();
        for id in ids {
            let r = self.store.message_ref(id)?.ok_or_else(|| not_found("message"))?;
            out.entry(r.account_id.clone()).or_default().push(r);
        }
        Ok(out)
    }

    fn uses_labels(&self, account_id: &str) -> bool {
        self.providers_for(account_id).ok().and_then(|p| p.mail.as_ref().map(|m| m.uses_labels())).unwrap_or(false)
    }

    pub(crate) fn set_flags(&self, ids: &[String], seen: Option<bool>, flagged: Option<bool>, actor: &str) -> Result<()> {
        for (account, refs) in self.group_by_account(ids)? {
            let mut changed = vec![];
            for r in &refs {
                let mut f = r.flags.clone();
                if let Some(s) = seen {
                    f.seen = s;
                }
                if let Some(fl) = flagged {
                    f.flagged = fl;
                }
                if f != r.flags {
                    self.store.set_flags(&r.id, &f)?;
                    changed.push(r.id.clone());
                }
            }
            if !changed.is_empty() {
                self.enqueue(&account, "flags", json!({ "message_ids": changed, "seen": seen, "flagged": flagged }), actor)?;
                self.store.recount_mailboxes(&account)?;
                self.emit(CoreEvent::MessagesChanged { account_id: account });
            }
        }
        Ok(())
    }

    pub(crate) fn move_messages(&self, ids: &[String], mailbox_id: &str, actor: &str) -> Result<()> {
        let target = self.store.get_mailbox(mailbox_id)?.ok_or_else(|| not_found("mailbox"))?;
        let target_remote = self.store.mailbox_remote_id(mailbox_id)?.ok_or_else(|| not_found("mailbox"))?;
        for (account, refs) in self.group_by_account(ids)? {
            if account != target.account_id {
                return Err(invalid("cannot move messages between accounts"));
            }
            if self.uses_labels(&account) {
                let inbox = self.store.mailbox_by_role(&account, MailboxRole::Inbox)?;
                let remove: Vec<String> = inbox.into_iter().map(|m| m.id).filter(|id| id != mailbox_id).collect();
                self.labels_inner(&account, refs, &[mailbox_id.to_string()], &remove, actor)?;
                continue;
            }
            let refs: Vec<MessageRef> = refs.into_iter().filter(|r| !r.mailboxes.iter().any(|(m, _, _)| m == mailbox_id)).collect();
            if refs.is_empty() {
                continue;
            }
            for r in &refs {
                self.store.set_memberships(&r.id, &[(mailbox_id.to_string(), None)])?;
            }
            self.enqueue(&account, "move", json!({ "refs": refs, "to_remote_mailbox": target_remote }), actor)?;
            self.store.recount_mailboxes(&account)?;
            self.emit(CoreEvent::MessagesChanged { account_id: account });
        }
        Ok(())
    }

    fn labels_inner(&self, account: &str, refs: Vec<MessageRef>, add: &[String], remove: &[String], actor: &str) -> Result<()> {
        let to_remote = |ids: &[String]| -> Result<Vec<String>> {
            ids.iter().map(|i| self.store.mailbox_remote_id(i)?.ok_or_else(|| not_found("mailbox"))).collect()
        };
        let add_r = to_remote(add)?;
        let remove_r = to_remote(remove)?;
        for r in &refs {
            let mut mbs: Vec<(String, Option<u32>)> = r.mailboxes.iter().filter(|(m, _, _)| !remove.contains(m)).map(|(m, _, u)| (m.clone(), *u)).collect();
            for a in add {
                if !mbs.iter().any(|(m, _)| m == a) {
                    mbs.push((a.clone(), None));
                }
            }
            self.store.set_memberships(&r.id, &mbs)?;
        }
        self.enqueue(account, "labels", json!({ "refs": refs, "add": add_r, "remove": remove_r }), actor)?;
        self.store.recount_mailboxes(account)?;
        self.emit(CoreEvent::MessagesChanged { account_id: account.to_string() });
        Ok(())
    }

    pub(crate) fn set_labels(&self, ids: &[String], add: &[String], remove: &[String], actor: &str) -> Result<()> {
        for (account, refs) in self.group_by_account(ids)? {
            if !self.uses_labels(&account) {
                if remove.is_empty() && add.len() == 1 {
                    // Folder accounts: "label" = move.
                    return self.move_messages(ids, &add[0], actor);
                }
                return Err(invalid("this account uses folders; move messages instead"));
            }
            self.labels_inner(&account, refs, add, remove, actor)?;
        }
        Ok(())
    }

    pub(crate) fn archive(&self, ids: &[String], actor: &str) -> Result<()> {
        for (account, refs) in self.group_by_account(ids)? {
            if self.uses_labels(&account) {
                let inbox = self.store.mailbox_by_role(&account, MailboxRole::Inbox)?.ok_or_else(|| not_found("inbox"))?;
                self.labels_inner(&account, refs, &[], &[inbox.id], actor)?;
                continue;
            }
            let archive = self
                .store
                .mailbox_by_role(&account, MailboxRole::Archive)?
                .or(self.store.list_mailboxes(Some(&account))?.into_iter().find(|m| m.name.eq_ignore_ascii_case("archive")))
                .ok_or_else(|| invalid("this account has no Archive folder"))?;
            let ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();
            self.move_messages(&ids, &archive.id, actor)?;
        }
        Ok(())
    }

    pub(crate) fn trash(&self, ids: &[String], actor: &str) -> Result<()> {
        for (account, refs) in self.group_by_account(ids)? {
            let trash = self.store.mailbox_by_role(&account, MailboxRole::Trash)?.ok_or_else(|| invalid("this account has no Trash folder"))?;
            let (in_trash, elsewhere): (Vec<MessageRef>, Vec<MessageRef>) =
                refs.into_iter().partition(|r| r.mailboxes.iter().all(|(m, _, _)| *m == trash.id) && !r.mailboxes.is_empty());
            if !in_trash.is_empty() {
                if actor != "user" {
                    return Err(forbidden("permanent deletion requires the user"));
                }
                for r in &in_trash {
                    self.store.delete_message(&r.id)?;
                    self.search.delete(&r.id);
                }
                self.enqueue(&account, "expunge", json!({ "refs": in_trash }), actor)?;
            }
            if !elsewhere.is_empty() {
                if self.uses_labels(&account) {
                    let remove: Vec<String> = self.store.list_mailboxes(Some(&account))?.into_iter().filter(|m| m.role == MailboxRole::Inbox).map(|m| m.id).collect();
                    self.labels_inner(&account, elsewhere, std::slice::from_ref(&trash.id), &remove, actor)?;
                } else {
                    let ids: Vec<String> = elsewhere.iter().map(|r| r.id.clone()).collect();
                    self.move_messages(&ids, &trash.id, actor)?;
                }
            }
            self.store.recount_mailboxes(&account)?;
            self.emit(CoreEvent::MessagesChanged { account_id: account });
        }
        Ok(())
    }

    pub(crate) fn save_draft(&self, mut draft: Draft, sync_to_server: bool) -> Result<String> {
        self.store.get_account(&draft.account_id)?.ok_or_else(|| not_found("account"))?;
        if draft.id.is_none() {
            draft.id = Some(new_id());
        }
        let id = self.store.save_draft(&draft)?;
        if sync_to_server && self.mode == Mode::Full {
            self.enqueue(&draft.account_id, "save_draft", json!({ "draft_id": id }), "user")?;
        }
        Ok(id)
    }

    fn delete_draft(&self, id: &str) -> Result<()> {
        let Some((d, _)) = self.store.get_draft(id)? else { return Ok(()) };
        self.store.delete_draft(id)?;
        if let Some(rid) = self.store.cursor(&d.account_id, &format!("draft:{id}"))? {
            self.enqueue(&d.account_id, "delete_draft", json!({ "remote_id": rid }), "user")?;
            self.store.delete_cursor(&d.account_id, &format!("draft:{id}"))?;
        }
        Ok(())
    }

    pub(crate) fn send(&self, draft: Draft, actor: &str) -> Result<String> {
        if actor != "user" {
            return Err(forbidden("only the user can send mail"));
        }
        let acc = self.store.get_account(&draft.account_id)?.ok_or_else(|| not_found("account"))?;
        let rcpts: Vec<String> = draft.to.iter().chain(draft.cc.iter()).chain(draft.bcc.iter()).map(|a| a.email.trim().to_string()).filter(|e| !e.is_empty()).collect();
        if rcpts.is_empty() {
            return Err(invalid("add at least one recipient"));
        }
        for r in &rcpts {
            if !r.contains('@') || r.contains(char::is_whitespace) {
                return Err(invalid(format!("invalid address: {r}")));
            }
        }
        let raw = self.build_raw(&draft, false)?;
        let draft_id = match &draft.id {
            Some(id) if self.store.get_draft(id)?.is_some() => Some(id.clone()),
            _ => None,
        };
        let op = self.enqueue(
            &acc.id,
            "send",
            json!({
                "raw": base64::engine::general_purpose::STANDARD.encode(&raw),
                "rcpts": rcpts,
                "from": acc.email,
                "subject": draft.subject,
                "draft_id": draft_id,
                "reply_to_message": draft.in_reply_to,
            }),
            actor,
        )?;
        self.store.audit(actor, "send_queued", &json!({ "account": acc.email, "to": rcpts, "subject": draft.subject }))?;
        Ok(op)
    }

    // -- calendar writes ---------------------------------------------------------------

    fn visible_calendar_ids(&self, caller: &Caller, requested: Option<Vec<String>>) -> Result<Vec<String>> {
        let allowed = self.allowed_accounts(caller);
        let cals = self.store.list_calendars(None)?;
        Ok(cals
            .into_iter()
            .filter(|c| allowed.as_ref().is_none_or(|ids| ids.contains(&c.cal.account_id)))
            .filter(|c| match &requested {
                Some(r) => r.contains(&c.cal.id),
                None => c.cal.visible,
            })
            .map(|c| c.cal.id)
            .collect())
    }

    fn validate_event(&self, e: &Event) -> Result<crate::store::StoredCalendar> {
        if e.title.trim().is_empty() {
            return Err(invalid("event needs a title"));
        }
        if e.end < e.start {
            return Err(invalid("event ends before it starts"));
        }
        if let Some(tz) = &e.timezone {
            tz.parse::<chrono_tz::Tz>().map_err(|_| invalid("unknown timezone"))?;
        }
        let cal = self.store.get_calendar(&e.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        if cal.cal.read_only {
            return Err(invalid("calendar is read-only"));
        }
        Ok(cal)
    }

    pub(crate) fn create_event(&self, mut e: Event, actor: &str) -> Result<Event> {
        let cal = self.validate_event(&e)?;
        let acc = self.store.get_account(&cal.cal.account_id)?.ok_or_else(|| not_found("account"))?;
        e.uid = Some(e.uid.clone().unwrap_or_else(satchel_ical::new_uid));
        if !e.attendees.is_empty() && e.organizer.is_none() {
            e.organizer = Some(Address { name: Some(acc.display_name.clone()), email: acc.email.clone() });
        }
        let ics = satchel_ical::serialize(&e, &Default::default());
        let parsed = satchel_ical::parse(&ics);
        let range = crate::calendar::series_range(&parsed.events);
        let id = self.store.upsert_event(&cal.cal.id, &acc.id, &format!("local:{}", new_id()), None, e.uid.as_deref().unwrap(), &ics, &e.title, range, e.rrule.is_some(), true)?;
        self.enqueue(&acc.id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "event_created", &json!({ "title": e.title, "start": e.start }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: acc.id.clone() });
        e.id = Some(id);
        Ok(e)
    }

    pub(crate) fn update_event(&self, mut e: Event, actor: &str) -> Result<Event> {
        let id = e.id.clone().ok_or_else(|| invalid("event id required"))?;
        let stored = self.store.get_event(&id)?.ok_or_else(|| not_found("event"))?;
        if stored.calendar_id != e.calendar_id {
            return Err(invalid("moving events between calendars is not supported; create a copy instead"));
        }
        self.validate_event(&e)?;
        let old = satchel_ical::parse(&stored.ics);
        let master = old.events.iter().find(|x| x.recurrence_id.is_none());
        let exdates: Vec<chrono::DateTime<Utc>> = master.map(|m| m.exdates.iter().map(|x| x.to_utc()).collect()).unwrap_or_default();
        let sequence = master.map(|m| m.sequence + 1).unwrap_or(1);
        e.uid = Some(stored.uid.clone());
        let overrides = if e.rrule.is_some() { satchel_ical::override_blocks(&stored.ics) } else { vec![] };
        let ics = satchel_ical::serialize(&e, &satchel_ical::SerializeOpts { sequence, exdates: &exdates, extra_components: &overrides, ..Default::default() });
        let parsed = satchel_ical::parse(&ics);
        let range = crate::calendar::series_range(&parsed.events);
        self.store.update_event_ics(&id, &ics, &e.title, range, e.rrule.is_some())?;
        self.enqueue(&stored.account_id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "event_updated", &json!({ "title": e.title, "start": e.start }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: stored.account_id.clone() });
        Ok(e)
    }

    pub(crate) fn delete_event(&self, id: &str, actor: &str) -> Result<()> {
        let stored = self.store.get_event(id)?.ok_or_else(|| not_found("event"))?;
        let cal = self.store.get_calendar(&stored.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        if cal.cal.read_only {
            return Err(invalid("calendar is read-only"));
        }
        self.store.delete_event(id)?;
        self.enqueue(&stored.account_id, "cal_delete", json!({ "calendar_remote_id": cal.remote_id, "remote_id": stored.remote_id, "etag": stored.etag }), actor)?;
        self.store.audit(actor, "event_deleted", &json!({ "uid": stored.uid }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: stored.account_id });
        Ok(())
    }

    pub(crate) fn respond(&self, id: &str, response: PartStat, actor: &str) -> Result<()> {
        let stored = self.store.get_event(id)?.ok_or_else(|| not_found("event"))?;
        let acc = self.store.get_account(&stored.account_id)?.ok_or_else(|| not_found("account"))?;
        let parsed = satchel_ical::parse(&stored.ics);
        if !parsed.events.iter().any(|e| e.attendees.iter().any(|a| a.email.eq_ignore_ascii_case(&acc.email))) {
            return Err(invalid("you are not an attendee of this event"));
        }
        let ics = satchel_ical::set_partstat(&stored.ics, &acc.email, response);
        let reparsed = satchel_ical::parse(&ics);
        let range = crate::calendar::series_range(&reparsed.events);
        let title = reparsed.events.first().map(|e| e.summary.clone()).unwrap_or_default();
        self.store.update_event_ics(id, &ics, &title, range, reparsed.events.iter().any(|e| e.rrule.is_some()))?;
        self.enqueue(&acc.id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "invite_response", &json!({ "event": title, "response": satchel_ical::partstat_str(response) }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: acc.id });
        Ok(())
    }

    // -- proposals ---------------------------------------------------------------

    fn create_proposal(&self, agent: &Agent, action: ProposalAction, rationale: String, source_thread_ids: Vec<String>) -> Result<Proposal> {
        let hour_ago = Utc::now() - Duration::hours(1);
        if self.store.proposals_since(&agent.id, hour_ago)? >= agent.policy.max_proposals_per_hour {
            return Err(err(ErrorCode::RateLimited, "proposal rate limit reached; try again later"));
        }
        // Agents retry; an identical pending proposal is returned instead of duplicated.
        let wanted = serde_json::to_value(&action)?;
        for existing in self.store.list_proposals(Some(ProposalStatus::Pending), Some(&agent.id))? {
            if serde_json::to_value(&existing.action)? == wanted {
                return Ok(existing);
            }
        }
        let caller = Caller::Agent(agent.clone());
        for t in &source_thread_ids {
            let ts = self.store.get_thread_summary(t)?.ok_or_else(|| not_found("source thread"))?;
            self.check_account(&caller, &ts.account_id)?;
        }
        let mut versions: HashMap<String, i64> = HashMap::new();
        let mut warnings: Vec<String> = vec![];
        let mut check_msgs = |ids: &[String]| -> Result<()> {
            if ids.is_empty() {
                return Err(invalid("no messages given"));
            }
            for id in ids {
                let m = self.check_message(&caller, id)?;
                versions.insert(format!("msg:{id}"), m.version);
            }
            Ok(())
        };
        let summary = match &action {
            ProposalAction::Draft { draft } => {
                self.check_account(&caller, &draft.account_id)?;
                self.store.get_account(&draft.account_id)?.ok_or_else(|| not_found("account"))?;
                let all: Vec<&Address> = draft.to.iter().chain(draft.cc.iter()).chain(draft.bcc.iter()).collect();
                if all.is_empty() {
                    return Err(invalid("a draft needs at least one recipient"));
                }
                // Participants of the conversation this draft responds to.
                let mut known: HashSet<String> = HashSet::new();
                let mut threads = source_thread_ids.clone();
                if let Some(irt) = &draft.in_reply_to {
                    let m = self.check_message(&caller, irt)?;
                    versions.insert(format!("msg:{irt}"), m.version);
                    threads.push(m.summary.thread_id.clone());
                }
                for t in &threads {
                    for id in self.store.thread_message_ids(t)? {
                        if let Some(m) = self.store.message_row(&id)? {
                            for a in m.summary.from.iter().chain(m.summary.to.iter()).chain(m.cc.iter()).chain(m.reply_to.iter()) {
                                known.insert(a.email.to_lowercase());
                            }
                        }
                    }
                }
                for a in &all {
                    if threads.is_empty() {
                        warnings.push(format!("New conversation with {}", a.email));
                    } else if !known.contains(&a.email.to_lowercase()) {
                        warnings.push(format!("{} was not part of the original conversation", a.email));
                    }
                }
                if !draft.bcc.is_empty() {
                    warnings.push("Includes Bcc recipients".into());
                }
                if all.len() > 10 {
                    warnings.push(format!("{} recipients", all.len()));
                }
                if !draft.attachments.is_empty() {
                    warnings.push(format!("{} attachment(s) added by the agent", draft.attachments.len()));
                }
                if draft.forward_of.is_some() {
                    warnings.push("Forwards an existing message (with its attachments)".into());
                }
                format!("Draft to {}: {}", all.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", "), draft.subject)
            }
            ProposalAction::Labels { message_ids, add, remove } => {
                check_msgs(message_ids)?;
                for mb in add.iter().chain(remove.iter()) {
                    self.store.get_mailbox(mb)?.ok_or_else(|| not_found("mailbox"))?;
                }
                format!("Change labels on {} message(s)", message_ids.len())
            }
            ProposalAction::Archive { message_ids } => {
                check_msgs(message_ids)?;
                format!("Archive {} message(s)", message_ids.len())
            }
            ProposalAction::Move { message_ids, mailbox_id } => {
                check_msgs(message_ids)?;
                let mb = self.store.get_mailbox(mailbox_id)?.ok_or_else(|| not_found("mailbox"))?;
                if mb.role == MailboxRole::Trash {
                    warnings.push("Moves messages to Trash".into());
                }
                format!("Move {} message(s) to {}", message_ids.len(), mb.name)
            }
            ProposalAction::MarkRead { message_ids, seen } => {
                check_msgs(message_ids)?;
                format!("Mark {} message(s) as {}", message_ids.len(), if *seen { "read" } else { "unread" })
            }
            ProposalAction::Flag { message_ids, flagged } => {
                check_msgs(message_ids)?;
                format!("{} {} message(s)", if *flagged { "Flag" } else { "Unflag" }, message_ids.len())
            }
            ProposalAction::CreateEvent { event } => {
                self.check_calendar_access(&caller)?;
                let cal = self.validate_event(event)?;
                self.check_account(&caller, &cal.cal.account_id)?;
                if !event.attendees.is_empty() {
                    warnings.push(format!("Sends invitations to {}", event.attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")));
                }
                format!("New event: {} ({})", event.title, event.start.format("%a %b %-d %H:%M UTC"))
            }
            ProposalAction::UpdateEvent { event } => {
                self.check_calendar_access(&caller)?;
                let id = event.id.clone().ok_or_else(|| invalid("event id required"))?;
                let stored = self.store.get_event(&id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id)?;
                self.validate_event(event)?;
                versions.insert(format!("event:{id}"), stored.version);
                format!("Change event: {} ({})", event.title, event.start.format("%a %b %-d %H:%M UTC"))
            }
            ProposalAction::CancelEvent { event_id } => {
                self.check_calendar_access(&caller)?;
                let stored = self.store.get_event(event_id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id)?;
                versions.insert(format!("event:{event_id}"), stored.version);
                let title = crate::calendar::get_event(&self.store, event_id)?.map(|e| e.title).unwrap_or_default();
                warnings.push("Deletes the event (and notifies attendees if you organize it)".into());
                format!("Cancel event: {title}")
            }
            ProposalAction::RespondToInvite { event_id, response } => {
                self.check_calendar_access(&caller)?;
                let stored = self.store.get_event(event_id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id)?;
                versions.insert(format!("event:{event_id}"), stored.version);
                let title = crate::calendar::get_event(&self.store, event_id)?.map(|e| e.title).unwrap_or_default();
                format!("{} invitation: {title}", match response {
                    PartStat::Accepted => "Accept",
                    PartStat::Declined => "Decline",
                    PartStat::Tentative => "Tentatively accept",
                    PartStat::NeedsAction => "Reset response to",
                })
            }
        };
        let p = Proposal {
            id: new_id(),
            agent_id: agent.id.clone(),
            agent_name: agent.name.clone(),
            created_at: Utc::now(),
            status: ProposalStatus::Pending,
            action,
            rationale: satchel_mime::truncate_chars(&rationale, 2000),
            source_thread_ids,
            warnings,
            review_note: None,
            decided_at: None,
            summary,
        };
        self.store.insert_proposal(&p, &versions)?;
        self.store.audit(&caller.actor(), "proposal_created", &json!({ "proposal": p.id, "summary": p.summary }))?;
        self.emit(CoreEvent::ProposalsChanged);
        Ok(p)
    }

    async fn approve_proposal(&self, id: &str, edited: Option<ProposalAction>) -> Result<Value> {
        let (p, versions) = self.store.get_proposal(id)?.ok_or_else(|| not_found("proposal"))?;
        if !matches!(p.status, ProposalStatus::Pending | ProposalStatus::Stale) {
            return Err(err(ErrorCode::Conflict, format!("proposal is already {:?}", p.status).to_lowercase()));
        }
        // Staleness: targets changed since the agent looked at them.
        if p.status == ProposalStatus::Pending && edited.is_none() {
            for (k, v) in &versions {
                let current = if let Some(mid) = k.strip_prefix("msg:") {
                    self.store.message_version(mid)?
                } else if let Some(eid) = k.strip_prefix("event:") {
                    self.store.get_event(eid)?.map(|e| e.version)
                } else {
                    None
                };
                if current != Some(*v) {
                    self.store.set_proposal_status(id, ProposalStatus::Stale, Some("Target changed after the proposal was made"), None)?;
                    self.emit(CoreEvent::ProposalsChanged);
                    return Err(err(ErrorCode::Conflict, "the message or event changed since this was proposed; review it again"));
                }
            }
        }
        let action = edited.clone().unwrap_or(p.action.clone());
        let actor = format!("user (approved {})", p.agent_name);
        let mut result = json!({ "ok": true });
        let applied: Result<()> = (|| {
            match &action {
                ProposalAction::Draft { draft } => {
                    let sync = self.settings().sync_ai_drafts_to_server;
                    let mut d = draft.clone();
                    d.id = None;
                    let draft_id = self.save_draft(d, sync)?;
                    result = json!({ "ok": true, "draft_id": draft_id });
                }
                ProposalAction::Labels { message_ids, add, remove } => self.set_labels(message_ids, add, remove, &actor)?,
                ProposalAction::Archive { message_ids } => self.archive(message_ids, &actor)?,
                ProposalAction::Move { message_ids, mailbox_id } => self.move_messages(message_ids, mailbox_id, &actor)?,
                ProposalAction::MarkRead { message_ids, seen } => self.set_flags(message_ids, Some(*seen), None, &actor)?,
                ProposalAction::Flag { message_ids, flagged } => self.set_flags(message_ids, None, Some(*flagged), &actor)?,
                ProposalAction::CreateEvent { event } => {
                    let e = self.create_event(event.clone(), &actor)?;
                    result = json!({ "ok": true, "event_id": e.id });
                }
                ProposalAction::UpdateEvent { event } => {
                    self.update_event(event.clone(), &actor)?;
                }
                ProposalAction::CancelEvent { event_id } => self.delete_event(event_id, &actor)?,
                ProposalAction::RespondToInvite { event_id, response } => self.respond(event_id, *response, &actor)?,
            }
            Ok(())
        })();
        match applied {
            Ok(()) => {
                self.store.set_proposal_status(id, ProposalStatus::Applied, None, edited.as_ref())?;
                self.store.audit("user", "proposal_approved", &json!({ "proposal": id, "agent": p.agent_name, "summary": p.summary, "edited": edited.is_some() }))?;
            }
            Err(e) => {
                self.store.set_proposal_status(id, ProposalStatus::Failed, Some(&format!("{e:#}")), None)?;
                self.emit(CoreEvent::ProposalsChanged);
                return Err(e);
            }
        }
        self.emit(CoreEvent::ProposalsChanged);
        Ok(result)
    }
}
