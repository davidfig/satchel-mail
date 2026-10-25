//! Changing mail: flags, moves, labels, trash.

use std::collections::HashMap;

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::Engine;
use crate::error::{forbidden, invalid, not_found};
use crate::store::MessageRef;

impl Engine {
    pub(crate) fn group_by_account(&self, ids: &[String]) -> Result<HashMap<String, Vec<MessageRef>>> {
        let mut out: HashMap<String, Vec<MessageRef>> = HashMap::new();
        for id in ids {
            let r = self.store.message_ref(id)?.ok_or_else(|| not_found("message"))?;
            out.entry(r.account_id.clone()).or_default().push(r);
        }
        Ok(out)
    }

    pub(crate) fn uses_labels(&self, account_id: &str) -> bool {
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
                // Moving takes it out of the Inbox, and out of Trash and Spam
                // too: left there, Gmail would still delete it after 30 days.
                let mut remove = vec![];
                for role in [MailboxRole::Inbox, MailboxRole::Trash, MailboxRole::Junk] {
                    remove.extend(self.store.mailbox_by_role(&account, role)?.map(|m| m.id).filter(|id| id != mailbox_id));
                }
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
            self.enqueue(&account, "move", json!({ "refs": refs, "to_remote_mailbox": target_remote, "to_mailbox": mailbox_id }), actor)?;
            self.store.recount_mailboxes(&account)?;
            self.emit(CoreEvent::MessagesChanged { account_id: account });
        }
        Ok(())
    }

    pub(crate) fn labels_inner(&self, account: &str, refs: Vec<MessageRef>, add: &[String], remove: &[String], actor: &str) -> Result<()> {
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
        self.enqueue(account, "labels", json!({ "refs": refs, "add": add_r, "remove": remove_r, "add_ids": add, "remove_ids": remove }), actor)?;
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

    /// Mark everything unread in a mailbox (or every mailbox with a role) read.
    pub(crate) fn mark_all_read(&self, mailbox_id: Option<&str>, role: Option<MailboxRole>, actor: &str) -> Result<Vec<String>> {
        if mailbox_id.is_none() && role.is_none() {
            return Err(invalid("give a mailbox_id or a role"));
        }
        let ids = self.store.unread_ids(mailbox_id, role)?;
        self.set_flags(&ids, Some(true), None, actor)?;
        Ok(ids)
    }
}
