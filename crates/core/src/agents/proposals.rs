//! Proposals: what agents suggest, and approving or rejecting it.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::{Caller, Engine};
use crate::error::{err, invalid, not_found};
use crate::store::new_id;
use crate::mail::drafts::check_addresses;

impl Engine {
    pub(crate) fn create_proposal(&self, agent: &Agent, action: ProposalAction, rationale: String, source_thread_ids: Vec<String>) -> Result<Proposal> {
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
                check_addresses(draft, true)?;
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
            ProposalAction::SaveContact { contact } => {
                if let Some(id) = &contact.id {
                    // Also checks the agent may see contacts, and this one.
                    self.get_contact(&caller, id)?;
                } else {
                    self.list_address_books(&caller)?;
                }
                let (w, version) = self.contact_proposal_warnings(contact)?;
                warnings.extend(w);
                if let (Some(id), Some(v)) = (&contact.id, version) {
                    versions.insert(format!("contact:{id}"), v);
                }
                let who = if contact.fields.name.trim().is_empty() {
                    contact.fields.emails.first().map(|e| e.email.clone()).unwrap_or_default()
                } else {
                    contact.fields.name.trim().to_string()
                };
                format!("{} contact: {who}", if contact.id.is_some() { "Update" } else { "New" })
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

    /// Keep the proposal's file in the AI folder, if it came from one, current.
    pub(crate) fn proposal_decided(&self, id: &str, edited: bool) {
        let mirror = self.settings().mirror;
        if mirror.enabled
            && let Ok(Some((p, _))) = self.store.get_proposal(id) {
                crate::ai_folder::import::update_status(&mirror, &p, edited);
            }
    }

    pub(crate) async fn approve_proposal(&self, id: &str, edited: Option<ProposalAction>) -> Result<Value> {
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
                } else if let Some(cid) = k.strip_prefix("contact:") {
                    self.store.get_contact(cid)?.filter(|c| !c.deleted).map(|c| c.version)
                } else {
                    None
                };
                if current != Some(*v) {
                    self.store.set_proposal_status(id, ProposalStatus::Stale, Some("Target changed after the proposal was made"), None)?;
                    self.proposal_decided(id, false);
                    self.emit(CoreEvent::ProposalsChanged);
                    return Err(err(ErrorCode::Conflict, "the message, event or contact changed since this was proposed; review it again"));
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
                ProposalAction::SaveContact { contact } => {
                    let c = self.save_contact(contact.clone(), None, &actor)?;
                    result = json!({ "ok": true, "contact_id": c.id });
                }
            }
            Ok(())
        })();
        match applied {
            Ok(()) => {
                self.store.set_proposal_status(id, ProposalStatus::Applied, None, edited.as_ref())?;
                self.proposal_decided(id, edited.is_some());
                self.store.audit("user", "proposal_approved", &json!({ "proposal": id, "agent": p.agent_name, "summary": p.summary, "edited": edited.is_some() }))?;
            }
            Err(e) => {
                self.store.set_proposal_status(id, ProposalStatus::Failed, Some(&format!("{e:#}")), None)?;
                self.proposal_decided(id, false);
                self.emit(CoreEvent::ProposalsChanged);
                return Err(e);
            }
        }
        self.emit(CoreEvent::ProposalsChanged);
        Ok(result)
    }

    /// Proposals the caller may see: an agent sees only its own.
    pub(crate) fn list_proposals(&self, caller: &Caller, status: Option<ProposalStatus>) -> Result<Vec<Proposal>> {
        let agent = match caller {
            Caller::Agent(a) => Some(a.id.clone()),
            Caller::User => None,
        };
        self.store.list_proposals(status, agent.as_deref())
    }

    pub(crate) fn get_proposal(&self, caller: &Caller, proposal_id: &str) -> Result<Proposal> {
        let (p, _) = self.store.get_proposal(proposal_id)?.ok_or_else(|| not_found("proposal"))?;
        if let Caller::Agent(a) = caller
            && p.agent_id != a.id {
                return Err(not_found("proposal"));
            }
        Ok(p)
    }

    pub(crate) fn reject_proposal(&self, proposal_id: &str, note: Option<String>) -> Result<()> {
        let (p, _) = self.store.get_proposal(proposal_id)?.ok_or_else(|| not_found("proposal"))?;
        self.store.set_proposal_status(proposal_id, ProposalStatus::Rejected, note.as_deref(), None)?;
        self.proposal_decided(proposal_id, false);
        self.store.audit("user", "proposal_rejected", &json!({ "proposal": proposal_id, "agent": p.agent_name, "summary": p.summary, "note": note }))?;
        self.emit(CoreEvent::ProposalsChanged);
        Ok(())
    }
}
