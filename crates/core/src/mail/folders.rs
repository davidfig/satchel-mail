//! Folders (and Gmail labels): create, rename, move and delete.
//!
//! Changes apply locally at once and reach the server through the op queue,
//! like message changes. A new folder gets a `PENDING_MAILBOX` remote id until
//! the server creates it, so messages can be moved into it straight away.
//! Deleting a folder moves its messages to Trash first, so nothing is lost
//! with it; a label is just removed from its messages.

use anyhow::{Context, Result};
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::{invalid, not_found};
use crate::provider::{Providers, ResolvedOp, SyncCtx};
use crate::store::{OpRow, PENDING_MAILBOX, new_id};

impl Engine {
    /// A folder the user may rename, move or delete: their own, not Inbox,
    /// Sent and the like.
    fn own_folder(&self, id: &str) -> Result<Mailbox> {
        let m = self.store.get_mailbox(id)?.ok_or_else(|| not_found("folder"))?;
        if m.role != MailboxRole::Custom {
            return Err(invalid(format!("{} is a system folder and can't be changed", m.name)));
        }
        Ok(m)
    }

    /// Check a new name and work out the full path under `parent`.
    fn folder_path(&self, account_id: &str, name: &str, parent: Option<&str>, moving: Option<&Mailbox>) -> Result<(String, Option<Mailbox>)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("give the folder a name"));
        }
        if name.contains('/') {
            return Err(invalid("folder names can't contain /; put the folder inside another one instead"));
        }
        let parent = match parent {
            Some(p) => {
                let p = self.own_folder(p)?;
                if p.account_id != account_id {
                    return Err(invalid("folders can only go inside folders of the same account"));
                }
                if let Some(m) = moving
                    && (p.id == m.id || p.path.starts_with(&format!("{}/", m.path)))
                {
                    return Err(invalid("a folder can't go inside itself"));
                }
                Some(p)
            }
            None => None,
        };
        let path = match &parent {
            Some(p) => format!("{}/{name}", p.path),
            None => name.to_string(),
        };
        let taken = self
            .store
            .list_mailboxes(Some(account_id))?
            .into_iter()
            .any(|m| m.path.eq_ignore_ascii_case(&path) && moving.is_none_or(|x| x.id != m.id));
        if taken {
            return Err(invalid(format!("there is already a folder called {path}")));
        }
        Ok((path, parent))
    }

    pub(crate) fn create_mailbox(&self, account_id: &str, name: &str, parent_id: Option<&str>, actor: &str) -> Result<Mailbox> {
        self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))?;
        let (path, parent) = self.folder_path(account_id, name, parent_id, None)?;
        let name = name.trim();
        let id = self.store.upsert_mailbox(account_id, &format!("{PENDING_MAILBOX}{}", new_id()), name, &path, MailboxRole::Custom, self.uses_labels(account_id))?;
        self.enqueue(account_id, "mailbox_create", json!({ "mailbox_id": id, "name": name, "parent_id": parent.map(|p| p.id) }), actor)?;
        self.store.audit(actor, "folder_created", &json!({ "path": path }))?;
        self.emit(CoreEvent::MailboxesChanged { account_id: account_id.to_string() });
        self.store.get_mailbox(&id)?.ok_or_else(|| not_found("folder"))
    }

    pub(crate) fn update_mailbox(&self, id: &str, name: &str, parent_id: Option<&str>, actor: &str) -> Result<Mailbox> {
        let m = self.own_folder(id)?;
        let (path, parent) = self.folder_path(&m.account_id, name, parent_id, Some(&m))?;
        if path == m.path && name.trim() == m.name {
            return Ok(m);
        }
        self.store.rename_mailbox(id, name.trim(), &path)?;
        self.enqueue(&m.account_id, "mailbox_update", json!({ "mailbox_id": id, "name": name.trim(), "parent_id": parent.map(|p| p.id), "old_path": m.path }), actor)?;
        self.store.audit(actor, "folder_changed", &json!({ "from": m.path, "to": path }))?;
        self.emit(CoreEvent::MailboxesChanged { account_id: m.account_id.clone() });
        self.store.get_mailbox(id)?.ok_or_else(|| not_found("folder"))
    }

    /// Returns how many messages moved to Trash.
    pub(crate) fn delete_mailbox(&self, id: &str, actor: &str) -> Result<usize> {
        let m = self.own_folder(id)?;
        let all = self.store.list_mailboxes(Some(&m.account_id))?;
        if all.iter().any(|x| x.path.starts_with(&format!("{}/", m.path))) {
            return Err(invalid(format!("{} has folders inside it; move or delete those first", m.name)));
        }
        let ids = self.store.mailbox_message_ids(id)?;
        let mut moved = 0;
        if !ids.is_empty() && !m.is_label {
            let trash = self.store.mailbox_by_role(&m.account_id, MailboxRole::Trash)?.ok_or_else(|| invalid("this account has no Trash folder to move the messages to"))?;
            self.move_messages(&ids, &trash.id, actor)?;
            moved = ids.len();
        }
        let remote = self.store.mailbox_remote_id(id)?.context("folder has no remote id")?;
        self.store.remove_mailboxes(&m.account_id, &[id.to_string()])?;
        self.store.recount_mailboxes(&m.account_id)?;
        self.enqueue(&m.account_id, "mailbox_delete", json!({ "remote_id": remote, "path": m.path }), actor)?;
        self.store.audit(actor, "folder_deleted", &json!({ "path": m.path, "moved_to_trash": moved }))?;
        self.emit(CoreEvent::MailboxesChanged { account_id: m.account_id.clone() });
        if !ids.is_empty() {
            self.emit(CoreEvent::MessagesChanged { account_id: m.account_id.clone() });
        }
        Ok(moved)
    }

    /// The server's id for a local folder, failing (to retry later) while the
    /// folder's own create hasn't reached the server yet.
    fn live_remote_id(&self, mailbox_id: &str) -> Result<String> {
        let rid = self.store.mailbox_remote_id(mailbox_id)?.ok_or_else(|| not_found("folder"))?;
        if rid.starts_with(PENDING_MAILBOX) {
            return Err(crate::error::err(ErrorCode::Network, "the folder hasn't been created on the server yet"));
        }
        Ok(rid)
    }

    /// Run a queued `mailbox_*` op.
    pub(crate) async fn run_mailbox_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let mail = providers.mail.as_ref().context("account has no mail provider")?;
        let p = &op.payload;
        let str_of = |k: &str| p.get(k).and_then(Value::as_str);
        let parent = match str_of("parent_id") {
            Some(pid) => Some(self.live_remote_id(pid)?),
            None => None,
        };
        match op.kind.as_str() {
            "mailbox_create" => {
                let id = str_of("mailbox_id").context("mailbox_id")?;
                // Deleted again before it reached the server.
                let Some(m) = self.store.get_mailbox(id)? else { return Ok(()) };
                let out = mail.apply(ctx, &ResolvedOp::CreateMailbox { name: m.name.clone(), parent, path: m.path.clone() }).await?;
                let rid = out.mailbox_remote_id.context("provider returned no folder id")?;
                self.store.set_mailbox_remote_id(id, &rid)?;
            }
            "mailbox_update" => {
                let id = str_of("mailbox_id").context("mailbox_id")?;
                let Some(m) = self.store.get_mailbox(id)? else { return Ok(()) };
                let remote_id = self.live_remote_id(id)?;
                let resolved = ResolvedOp::UpdateMailbox {
                    remote_id,
                    old_path: str_of("old_path").unwrap_or_default().to_string(),
                    name: str_of("name").unwrap_or(&m.name).to_string(),
                    parent,
                    path: m.path.clone(),
                };
                let out = mail.apply(ctx, &resolved).await?;
                for (old, new) in out.mailbox_renamed {
                    if let Some(x) = self.store.mailbox_by_remote(&ctx.account.id, &old)? {
                        self.store.set_mailbox_remote_id(&x.id, &new)?;
                    }
                    // Message ids name their folder too (IMAP).
                    self.store.rekey_mailbox_messages(&ctx.account.id, &old, &new)?;
                }
            }
            "mailbox_delete" => {
                let rid = str_of("remote_id").context("remote_id")?;
                // Never reached the server: nothing to delete there.
                if rid.starts_with(PENDING_MAILBOX) {
                    return Ok(());
                }
                mail.apply(ctx, &ResolvedOp::DeleteMailbox { remote_id: rid.to_string(), path: str_of("path").unwrap_or_default().to_string() }).await?;
            }
            other => anyhow::bail!("unknown folder op {other}"),
        }
        self.store.recount_mailboxes(&ctx.account.id)?;
        self.emit(CoreEvent::MailboxesChanged { account_id: ctx.account.id.clone() });
        Ok(())
    }
}
