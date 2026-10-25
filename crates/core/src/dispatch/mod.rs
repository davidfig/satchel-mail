//! Command dispatch. Every client request lands in [`Engine::handle`].

mod access;

use std::sync::Arc;

use anyhow::Result;
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::{Caller, Engine, Mode};
use crate::error::{invalid, not_found, to_api};

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

    async fn dispatch(self: &Arc<Self>, caller: &Caller, cmd: Command) -> Result<Value> {
        match cmd {
            Command::Info => ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "mode": if self.mode == Mode::Full { "full" } else { "agent" },
                "data_dir": self.data_dir,
                "mirror_root": crate::ai_folder::root(&self.settings().mirror),
                "caller": caller.actor(),
            })),

            // ---------------------------------------------------------- accounts
            Command::ListAccounts => ok(self.list_accounts(caller)?),
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
                self.remove_account(&account_id)?;
                ok(json!({ "ok": true }))
            }
            Command::UpdateAccount { account_id, config, credential } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.update_account(&account_id, config, credential).await?)
            }
            Command::StartOAuth { provider } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(json!({ "auth_url": self.start_oauth(provider)? }))
            }
            Command::SyncNow { account_id } => {
                self.require_user(caller)?;
                self.sync_now(account_id)?;
                ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------------- mail reads
            Command::ListMailboxes { account_id } => ok(self.list_mailboxes(caller, account_id)?),
            Command::ListThreads { query } => ok(self.list_threads(caller, &query)?),
            Command::CountThreads { query } => {
                let f = self.thread_filter(caller, &query)?;
                ok(json!({ "total": self.store.count_threads(&f)? }))
            }
            Command::GetThread { thread_id } => ok(self.get_thread(caller, thread_id).await?),
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
                self.get_attachment(caller, &message_id, &attachment_id).await
            }
            Command::LoadRemoteImages { message_id, always_for_sender } => {
                self.require_user(caller)?;
                ok(self.load_remote_images(&message_id, always_for_sender).await?)
            }

            // ---------------------------------------------------------- mail writes
            Command::SetFlags { message_ids, seen, flagged } => {
                self.require_full()?;
                self.set_flags(&message_ids, seen, flagged, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::CreateMailbox { account_id, name, parent_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.create_mailbox(&account_id, &name, parent_id.as_deref(), &caller.actor())?)
            }
            Command::UpdateMailbox { mailbox_id, name, parent_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.update_mailbox(&mailbox_id, &name, parent_id.as_deref(), &caller.actor())?)
            }
            Command::DeleteMailbox { mailbox_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(json!({ "moved_to_trash": self.delete_mailbox(&mailbox_id, &caller.actor())? }))
            }
            Command::MarkAllRead { mailbox_id, role } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(json!({ "message_ids": self.mark_all_read(mailbox_id.as_deref(), role, &caller.actor())? }))
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
            Command::MarkSpam { message_ids } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.mark_spam(&message_ids, &caller.actor())?)
            }
            Command::MarkNotSpam { message_ids } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.mark_not_spam(&message_ids, &caller.actor())?)
            }
            Command::ListSenderRules => {
                self.require_user(caller)?;
                ok(self.store.list_sender_rules()?)
            }
            Command::SetSenderRule { email, verdict } => {
                self.require_user(caller)?;
                self.require_full()?;
                self.set_sender_rule(&email, verdict)?;
                ok(json!({ "ok": true }))
            }
            Command::Unsubscribe { message_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.unsubscribe(&message_id).await?)
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
            Command::EditDraft { message_id } => {
                self.require_full()?;
                ok(self.edit_draft(&message_id).await?)
            }
            Command::Send { draft } => {
                self.require_full()?;
                ok(json!({ "op_id": self.send(draft, &caller.actor())? }))
            }
            Command::ComposeReply { message_id, mode } => ok(self.compose_reply(&message_id, mode).await?),
            Command::SuggestAddresses { prefix } => {
                self.require_user(caller)?;
                ok(self.suggest_addresses(prefix.trim())?)
            }
            Command::GetPerson { emails } => {
                self.require_user(caller)?;
                ok(self.get_person(&emails)?)
            }

            // ---------------------------------------------------------- calendar
            Command::ListCalendars => {
                self.check_calendar_access(caller)?;
                ok(self.list_calendars(caller)?)
            }
            Command::SetCalendarVisible { calendar_id, visible } => {
                self.store.set_calendar_visible(&calendar_id, visible)?;
                self.emit(CoreEvent::CalendarChanged { account_id: String::new() });
                ok(json!({ "ok": true }))
            }
            Command::ListEvents { from, to, calendar_ids } => {
                self.check_calendar_access(caller)?;
                ok(self.list_events(caller, from, to, calendar_ids)?)
            }
            Command::GetEvent { event_id } => {
                self.check_calendar_access(caller)?;
                ok(self.get_event(caller, &event_id)?)
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
                ok(self.find_free_time(caller, from, to, duration_minutes, day_start_hour, day_end_hour, timezone.as_deref())?)
            }

            // ---------------------------------------------------------- contacts
            Command::ListContacts { search, limit } => ok(self.list_contacts(caller, search.as_deref(), limit)?),
            Command::GetContact { contact_id } => ok(self.get_contact(caller, &contact_id)?),
            Command::ListAddressBooks => ok(self.list_address_books(caller)?),
            Command::SaveContact { contact, address_book_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.save_contact(contact, address_book_id.as_deref(), &caller.actor())?)
            }
            Command::DeleteContact { contact_id } => {
                self.require_user(caller)?;
                self.require_full()?;
                self.delete_contact(&contact_id, &caller.actor())?;
                ok(json!({ "ok": true }))
            }
            Command::MergeContacts { contact_ids } => {
                self.require_user(caller)?;
                self.require_full()?;
                ok(self.merge_contacts(&contact_ids, &caller.actor())?)
            }

            // ---------------------------------------------------------- proposals
            Command::ListProposals { status } => ok(self.list_proposals(caller, status)?),
            Command::GetProposal { proposal_id } => ok(self.get_proposal(caller, &proposal_id)?),
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
                self.reject_proposal(&proposal_id, note)?;
                ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------------- agents
            Command::ListAgents => {
                self.require_user(caller)?;
                ok(self.store.list_agents()?)
            }
            Command::CreateAgent { name, policy } => {
                self.require_user(caller)?;
                let (agent, token) = self.create_agent(&name, &policy)?;
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
                self.retry_op(&id)?;
                ok(json!({ "ok": true }))
            }
        }
    }
}
