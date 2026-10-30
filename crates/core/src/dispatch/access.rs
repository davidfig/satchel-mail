//! What a caller may see and do: agent policy checks.


use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;

use crate::engine::{Caller, Engine, Mode};
use crate::error::{err, forbidden, not_found};

/// The commands an agent may run: reads and proposals, nothing else. This is
/// a second, independent list next to `Command::allowed_for_agent`, so that
/// widening that one by mistake can't let an agent act directly. Never add a
/// command here that sends, deletes, changes mail, events, contacts or settings.
pub(crate) fn agent_may_run(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Info
            | Command::ListAccounts
            | Command::ListMailboxes { .. }
            | Command::ListThreads { .. }
            | Command::CountThreads { .. }
            | Command::GetThread { .. }
            | Command::GetMessage { .. }
            | Command::GetAttachment { .. }
            | Command::ListCalendars
            | Command::ListEvents { .. }
            | Command::GetEvent { .. }
            | Command::FindFreeTime { .. }
            | Command::ListContacts { .. }
            | Command::GetContact { .. }
            | Command::ListAddressBooks
            | Command::CreateProposal { .. }
            | Command::GetProposal { .. }
            | Command::ListProposals { .. }
    )
}

impl Engine {
    pub(crate) fn allowed_accounts(&self, caller: &Caller) -> Option<Vec<String>> {
        match caller {
            Caller::User => None,
            Caller::Agent(a) => a.policy.account_ids.clone(),
        }
    }

    /// Whether the caller may see `what` (a message, thread, event…) in
    /// this account. If not, the error is the one a missing `what` gets, so
    /// an agent can't tell an item it may not see from one that isn't there.
    pub(crate) fn check_account(&self, caller: &Caller, account_id: &str, what: &str) -> Result<()> {
        if let Some(ids) = self.allowed_accounts(caller)
            && !ids.iter().any(|x| x == account_id) {
                return Err(not_found(what));
            }
        Ok(())
    }

    pub(crate) fn cutoff(&self, caller: &Caller) -> Option<chrono::DateTime<Utc>> {
        match caller {
            Caller::Agent(a) => a.policy.since_days.map(|d| Utc::now() - Duration::days(d as i64)),
            Caller::User => None,
        }
    }

    pub(crate) fn check_calendar_access(&self, caller: &Caller) -> Result<()> {
        if let Caller::Agent(a) = caller
            && !a.policy.allow_calendar {
                return Err(forbidden("calendar access is not enabled for this agent"));
            }
        Ok(())
    }

    pub(crate) fn check_message(&self, caller: &Caller, id: &str) -> Result<crate::store::StoredMessage> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        self.check_account(caller, &m.summary.account_id, "message")?;
        if let Some(c) = self.cutoff(caller)
            && m.summary.date < c {
                return Err(not_found("message"));
            }
        Ok(m)
    }

    pub(crate) fn require_user(&self, caller: &Caller) -> Result<()> {
        match caller {
            Caller::User => Ok(()),
            Caller::Agent(_) => Err(forbidden("requires the user")),
        }
    }

    pub(crate) fn require_full(&self) -> Result<()> {
        if self.mode != Mode::Full {
            return Err(err(ErrorCode::Conflict, "this process is read-only; open Satchel Mail to make changes"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Even if `allowed_for_agent` let these through, `agent_may_run` refuses them.
    #[test]
    fn agents_may_not_run_writes() {
        let draft = serde_json::json!({ "account_id": "a", "to": [], "subject": "", "body_text": "" });
        let event = serde_json::json!({ "id": "e", "calendar_id": "c", "title": "t", "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z" });
        let ids = serde_json::json!(["m"]);
        let writes = [
            ("send", serde_json::json!({ "draft": draft })),
            ("save_draft", serde_json::json!({ "draft": draft })),
            ("delete_draft", serde_json::json!({ "draft_id": "d" })),
            ("trash", serde_json::json!({ "message_ids": ids })),
            ("archive", serde_json::json!({ "message_ids": ids })),
            ("move", serde_json::json!({ "message_ids": ids, "mailbox_id": "x" })),
            ("set_flags", serde_json::json!({ "message_ids": ids, "seen": true, "flagged": null })),
            ("set_labels", serde_json::json!({ "message_ids": ids, "add": [], "remove": [] })),
            ("create_event", serde_json::json!({ "event": event })),
            ("update_event", serde_json::json!({ "event": event })),
            ("delete_event", serde_json::json!({ "event_id": "e" })),
            ("set_calendar_visible", serde_json::json!({ "calendar_id": "c", "visible": true })),
            ("respond_to_invite", serde_json::json!({ "event_id": "e", "response": "accepted" })),
            ("approve_proposal", serde_json::json!({ "proposal_id": "p", "edited": null })),
            ("update_settings", serde_json::json!({ "settings": Settings::default() })),
            ("get_settings", serde_json::Value::Null),
            ("list_outbox", serde_json::Value::Null),
            ("list_drafts", serde_json::Value::Null),
        ];
        for (name, args) in writes {
            let v = if args.is_null() { serde_json::json!({ "cmd": name }) } else { serde_json::json!({ "cmd": name, "args": args }) };
            let cmd: Command = serde_json::from_value(v).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!agent_may_run(&cmd), "{name} must require the user");
        }
        assert!(agent_may_run(&Command::ListAccounts));
    }
}
