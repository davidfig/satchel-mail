//! Proposals: what agents suggest, and approving or rejecting it.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::{Caller, Engine};
use crate::error::{err, forbidden, invalid, not_found};
use crate::store::new_id;
use crate::mail::drafts::check_addresses;

impl Engine {
    pub(crate) fn create_proposal(&self, agent: &Agent, mut action: ProposalAction, rationale: String, source_thread_ids: Vec<String>) -> Result<Proposal> {
        // A new event gets its own UID and the account as organizer: reusing a
        // real meeting's UID could turn "New event" into an update of it.
        if let ProposalAction::CreateEvent { event } = &mut action {
            event.uid = None;
            event.organizer = None;
        }
        // Review shows only the plain text, so that is all an agent draft carries.
        if let ProposalAction::Draft { draft } = &mut action {
            draft.body_html = None;
            if let Some(a) = draft.attachments.iter().find(|a| !is_plain_mime_type(&a.mime_type)) {
                return Err(invalid(format!("attachment {:?} has an invalid MIME type; use a plain type/subtype", a.filename)));
            }
        }
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
        let cutoff = self.cutoff(&caller);
        for t in &source_thread_ids {
            // The same limits as reading the thread: its account, and the date range.
            let ts = self.store.get_thread_summary(t)?.ok_or_else(|| not_found("source thread"))?;
            self.check_account(&caller, &ts.account_id, "source thread")?;
            if cutoff.is_some_and(|c| ts.last_date < c) {
                return Err(not_found("source thread"));
            }
        }
        let mut versions: HashMap<String, i64> = HashMap::new();
        let mut warnings: Vec<String> = vec![];
        // Checks the messages and returns the accounts they're in.
        let mut check_msgs = |ids: &[String]| -> Result<HashSet<String>> {
            if ids.is_empty() {
                return Err(invalid("no messages given"));
            }
            let mut accounts = HashSet::new();
            for id in ids {
                let m = self.check_message(&caller, id)?;
                versions.insert(format!("msg:{id}"), m.version);
                accounts.insert(m.summary.account_id);
            }
            Ok(accounts)
        };
        // A mailbox the agent may use, in the same account as all the messages.
        let check_mailbox = |id: &str, accounts: &HashSet<String>| -> Result<Mailbox> {
            let mb = self.store.get_mailbox(id)?.ok_or_else(|| not_found("mailbox"))?;
            self.check_account(&caller, &mb.account_id, "mailbox")?;
            if accounts.iter().any(|a| *a != mb.account_id) {
                return Err(invalid("the mailbox is in a different account than the messages"));
            }
            Ok(mb)
        };
        let summary = match &action {
            ProposalAction::Draft { draft } => {
                self.check_account(&caller, &draft.account_id, "account")?;
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
                if !draft.attachments.is_empty() && !agent.policy.allow_attachments {
                    return Err(forbidden("attachments are not enabled for this agent"));
                }
                if let Some(fwd) = &draft.forward_of {
                    let m = self.check_message(&caller, fwd)?;
                    if m.summary.account_id != draft.account_id {
                        return Err(invalid("the forwarded message is in a different account than the draft"));
                    }
                    if !agent.policy.allow_attachments && m.attachments.iter().any(|a| !a.inline) {
                        return Err(forbidden("the forwarded message has attachments, and attachments are not enabled for this agent"));
                    }
                    versions.insert(format!("msg:{fwd}"), m.version);
                    threads.push(m.summary.thread_id.clone());
                }
                for t in &threads {
                    for id in self.store.thread_message_ids(t)? {
                        if let Some(m) = self.store.message_row(&id)?.filter(|m| cutoff.is_none_or(|c| m.summary.date >= c)) {
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
                let accounts = check_msgs(message_ids)?;
                let mut special: Vec<String> = vec![];
                for (id, added) in add.iter().map(|m| (m, true)).chain(remove.iter().map(|m| (m, false))) {
                    let mb = check_mailbox(id, &accounts)?;
                    let role = match mb.role {
                        MailboxRole::Trash => "Trash",
                        MailboxRole::Junk => "Junk",
                        MailboxRole::Sent => "Sent",
                        MailboxRole::Drafts => "Drafts",
                        MailboxRole::Archive => "Archive",
                        _ => continue,
                    };
                    special.push(format!("{} {role}", if added { "adds" } else { "removes" }));
                }
                if !special.is_empty() {
                    warnings.push(format!("Changes special mailboxes: {}", special.join(", ")));
                }
                // On a folder account a one-mailbox "label" is applied as a move.
                if accounts.iter().any(|a| !self.uses_labels(a)) && remove.is_empty() && add.len() == 1 {
                    warnings.push("Moves the messages out of their current folder (this account uses folders)".into());
                }
                format!("Change labels on {} message(s)", message_ids.len())
            }
            ProposalAction::Archive { message_ids } => {
                check_msgs(message_ids)?;
                format!("Archive {} message(s)", message_ids.len())
            }
            ProposalAction::Move { message_ids, mailbox_id } => {
                let accounts = check_msgs(message_ids)?;
                let mb = check_mailbox(mailbox_id, &accounts)?;
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
                // Scope first: a calendar the agent may not see is "not
                // found", never "read-only".
                let cal = self.store.get_calendar(&event.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
                self.check_account(&caller, &cal.cal.account_id, "calendar")?;
                self.validate_event(event)?;
                if !event.attendees.is_empty() {
                    warnings.push(format!("Sends invitations to {}", event.attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")));
                }
                format!("New event: {} ({})", event.title, event.start.format("%a %b %-d %H:%M UTC"))
            }
            ProposalAction::UpdateEvent { event } => {
                self.check_calendar_access(&caller)?;
                let id = event.id.clone().ok_or_else(|| invalid("event id required"))?;
                let stored = self.store.get_event(&id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id, "event")?;
                // Its own calendar, so naming another can't probe that one.
                if stored.calendar_id != event.calendar_id {
                    return Err(invalid("moving events between calendars is not supported; create a copy instead"));
                }
                self.validate_event(event)?;
                if event.uid.as_ref().is_some_and(|u| *u != stored.uid) {
                    return Err(invalid("an event's UID can't be changed"));
                }
                versions.insert(format!("event:{id}"), stored.version);
                let old = crate::calendar::get_event(&self.store, &id)?.ok_or_else(|| not_found("event"))?;
                warnings.extend(event_change_warnings(&old, event));
                format!("Change event: {} ({})", event.title, event.start.format("%a %b %-d %H:%M UTC"))
            }
            ProposalAction::CancelEvent { event_id } => {
                self.check_calendar_access(&caller)?;
                let stored = self.store.get_event(event_id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id, "event")?;
                versions.insert(format!("event:{event_id}"), stored.version);
                let title = crate::calendar::get_event(&self.store, event_id)?.map(|e| e.title).unwrap_or_default();
                warnings.push("Deletes the event (and notifies attendees if you organize it)".into());
                format!("Cancel event: {title}")
            }
            ProposalAction::RespondToInvite { event_id, response } => {
                self.check_calendar_access(&caller)?;
                let stored = self.store.get_event(event_id)?.ok_or_else(|| not_found("event"))?;
                self.check_account(&caller, &stored.account_id, "event")?;
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
                let (w, version) = self.contact_proposal_warnings(&caller, contact)?;
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
            ProposalAction::MergeContacts { contact_ids } => {
                let mut uniq: Vec<&String> = vec![];
                for id in contact_ids {
                    if !uniq.contains(&id) {
                        uniq.push(id);
                    }
                }
                if uniq.len() < 2 {
                    return Err(invalid("give at least two different contacts to merge"));
                }
                let mut all = vec![];
                for id in &uniq {
                    // Also checks the agent may see contacts, and each one.
                    let c = self.get_contact(&caller, id)?;
                    versions.insert(format!("contact:{id}"), c.version);
                    all.push(c);
                }
                let label = |c: &Contact| if c.fields.name.trim().is_empty() { c.fields.emails.first().map(|e| e.email.clone()).unwrap_or_default() } else { c.fields.name.trim().to_string() };
                let keep = label(&all[0]);
                let mut names: Vec<String> = vec![];
                for c in &all {
                    let n = c.fields.name.trim();
                    if !n.is_empty() && !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                        names.push(n.to_string());
                    }
                }
                if names.len() > 1 {
                    warnings.push(format!("The contacts have different names: {}", names.join(", ")));
                }
                let had: HashSet<String> = all[0].fields.emails.iter().map(|e| e.email.to_lowercase()).collect();
                let added: Vec<&str> = all[1..].iter().flat_map(|c| c.fields.emails.iter()).filter(|e| !had.contains(&e.email.to_lowercase())).map(|e| e.email.as_str()).collect();
                if !added.is_empty() {
                    warnings.push(format!("Adds {} to {keep}", added.join(", ")));
                }
                warnings.push("Merging can't be undone from Satchel".into());
                format!("Merge {} contacts into {keep}", all.len())
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
            && let Ok(Some(file)) = self.store.proposal_file(id)
            && let Ok(Some((p, _))) = self.store.get_proposal(id)
        {
            crate::ai_folder::import::update_status(&mirror, &p, &file, edited);
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
        let actor = format!("user (approved {} [{}])", p.agent_name, p.agent_id);
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
                // The same path as `Command::MergeContacts`; its user-only
                // check is this approval.
                ProposalAction::MergeContacts { contact_ids } => {
                    let c = self.merge_contacts(contact_ids, &actor)?;
                    result = json!({ "ok": true, "contact_id": c.id });
                }
            }
            Ok(())
        })();
        match applied {
            Ok(()) => {
                self.store.set_proposal_status(id, ProposalStatus::Applied, None, edited.as_ref())?;
                self.proposal_decided(id, edited.is_some());
                self.store.audit("user", "proposal_approved", &json!({ "proposal": id, "agent": p.agent_name, "agent_id": p.agent_id, "summary": p.summary, "edited": edited.is_some() }))?;
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
        // A decided proposal keeps its outcome: rejecting one already applied would misreport history.
        if !matches!(p.status, ProposalStatus::Pending | ProposalStatus::Stale) {
            return Err(err(ErrorCode::Conflict, format!("proposal is already {:?}", p.status).to_lowercase()));
        }
        self.store.set_proposal_status(proposal_id, ProposalStatus::Rejected, note.as_deref(), None)?;
        self.proposal_decided(proposal_id, false);
        self.store.audit("user", "proposal_rejected", &json!({ "proposal": proposal_id, "agent": p.agent_name, "agent_id": p.agent_id, "summary": p.summary, "note": note }))?;
        self.emit(CoreEvent::ProposalsChanged);
        Ok(())
    }
}

/// A bare `type/subtype` MIME token: no parameters, spaces or line breaks
/// that could add headers to the attachment's MIME part.
fn is_plain_mime_type(m: &str) -> bool {
    let token = |t: &str| !t.is_empty() && t.len() <= 127 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b));
    m.split_once('/').is_some_and(|(t, s)| token(t) && token(s))
}

/// What an event change does that the user should know before approving:
/// invitations sent or withdrawn, and a different organizer.
fn event_change_warnings(old: &Event, new: &Event) -> Vec<String> {
    let emails = |e: &Event| -> Vec<String> { e.attendees.iter().map(|a| a.email.to_lowercase()).collect() };
    let (before, after) = (emails(old), emails(new));
    let mut out = vec![];
    let added: Vec<&str> = after.iter().filter(|e| !before.contains(e)).map(|e| e.as_str()).collect();
    if !added.is_empty() {
        out.push(format!("Sends invitations to {}", added.join(", ")));
    }
    let removed: Vec<&str> = before.iter().filter(|e| !after.contains(e)).map(|e| e.as_str()).collect();
    if !removed.is_empty() {
        out.push(format!("Removes {}", removed.join(", ")));
    }
    let organizer = |e: &Event| e.organizer.as_ref().map(|a| a.email.to_lowercase());
    if organizer(old) != organizer(new) {
        out.push(match organizer(new) {
            Some(o) => format!("Changes the organizer to {o}"),
            None => "Removes the organizer".into(),
        });
    }
    out
}
