//! The durable op queue: optimistic local changes are recorded as ops and
//! pushed to providers by the account loop, with retry and backoff.

mod calendar;
mod contacts;
mod mail;

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use satchel_api::*;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::engine::Engine;
use crate::error::is_transient;
use crate::provider::{CalendarOp, ContactOp, Providers, ResolvedOp, SyncCtx};
use crate::store::{MessageRef, OpRow};

const MAX_ATTEMPTS: u32 = 8;

/// Cursor key prefixes that hold each provider's mail sync position, reset
/// to resync after a failed op. Not Gmail's history id: its full sync only
/// fetches missing messages, so dropping it would fix nothing and lose the
/// server changes made since.
const MAIL_SYNC_CURSORS: [&str; 3] = ["imap:", "graph:delta:", "jmap:email_state"];

impl Engine {
    /// Try a failed or waiting op again now.
    pub(crate) fn retry_op(&self, id: &str) -> Result<()> {
        self.store.op_reset(id)?;
        for a in self.store.list_accounts()? {
            self.wake_account(&a.id);
        }
        Ok(())
    }

    pub(crate) fn enqueue(&self, account_id: &str, kind: &str, payload: Value, actor: &str) -> Result<String> {
        let id = self.store.enqueue_op(account_id, kind, &payload, actor)?;
        self.emit(CoreEvent::OutboxChanged);
        self.wake_account(account_id);
        Ok(id)
    }

    pub(crate) async fn run_ops(&self, ctx: &SyncCtx, providers: &Providers) -> Result<()> {
        // One loop per account runs ops, so a send still marked in flight here
        // was cut off (crash, quit, account restart) and may have gone out.
        const INTERRUPTED: &str = "Satchel stopped while sending, so it may have gone out; check Sent before retrying";
        for op in self.store.fail_interrupted_sends(&ctx.account.id, INTERRUPTED)? {
            warn!(op = %op.kind, "send interrupted; not retrying");
            self.store.audit(&op.actor, "op_failed", &json!({ "kind": op.kind, "error": INTERRUPTED }))?;
            self.emit(CoreEvent::OpFailed { account_id: op.account_id.clone(), description: describe(&op), error: INTERRUPTED.into() });
            self.emit(CoreEvent::OutboxChanged);
        }
        loop {
            let ops = self.store.due_ops(&ctx.account.id)?;
            if ops.is_empty() {
                return Ok(());
            }
            let mut progressed = false;
            for op in ops {
                match self.run_op(ctx, providers, &op).await {
                    Ok(()) => {
                        self.store.op_done(&op.id)?;
                        progressed = true;
                    }
                    Err(e) => {
                        let attempts = op.attempts + 1;
                        let msg = format!("{e:#}");
                        if is_transient(&e) && attempts < MAX_ATTEMPTS {
                            warn!(op = %op.kind, attempts, "op failed, will retry: {msg}");
                            self.store.op_retry(&op.id, &msg, attempts)?;
                        } else {
                            warn!(op = %op.kind, "op failed permanently: {msg}");
                            self.store.op_failed(&op.id, &msg)?;
                            self.store.audit(&op.actor, "op_failed", &json!({ "kind": op.kind, "error": msg }))?;
                            self.emit(CoreEvent::OpFailed { account_id: op.account_id.clone(), description: describe(&op), error: msg });
                            // Local optimistic state may now disagree with the server: force a full resync.
                            // A folder the server never created: drop the local placeholder.
                            if op.kind == "mailbox_create"
                                && let Some(id) = op.payload.get("mailbox_id").and_then(serde_json::Value::as_str)
                                && self.store.mailbox_remote_id(id)?.is_some_and(|r| r.starts_with(crate::store::PENDING_MAILBOX))
                            {
                                self.store.remove_mailboxes(&ctx.account.id, &[id.to_string()])?;
                                self.emit(CoreEvent::MailboxesChanged { account_id: ctx.account.id.clone() });
                            }
                            // Resync only the kind of data the op touched. Draft
                            // cursors (which server copy a local draft replaces)
                            // and other bookkeeping are kept.
                            if op.kind.starts_with("contact_") {
                                for b in self.store.list_address_books(Some(&ctx.account.id))? {
                                    self.store.set_address_book_token(&b.book.id, None)?;
                                }
                            } else if op.kind.starts_with("cal_") {
                                for c in self.store.list_calendars(Some(&ctx.account.id))? {
                                    self.store.set_calendar_token(&c.cal.id, None)?;
                                }
                            } else if op.kind != "send" {
                                for prefix in MAIL_SYNC_CURSORS {
                                    self.store.delete_cursors_with_prefix(&ctx.account.id, prefix)?;
                                }
                            }
                        }
                        self.emit(CoreEvent::OutboxChanged);
                        // Sends are independent; everything else keeps ordering, so stop here.
                        if op.kind == "send" {
                            continue;
                        }
                        return Ok(());
                    }
                }
                self.emit(CoreEvent::OutboxChanged);
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    /// A mailbox's current remote id from its local id, else the remote id
    /// recorded when the op was queued. Waits (retries) while the folder is
    /// still being created on the server.
    pub(crate) fn target_remote(&self, local: Option<&Value>, recorded: &Value) -> Result<String> {
        if let Some(id) = local.and_then(Value::as_str)
            && let Some(rid) = self.store.mailbox_remote_id(id)?
        {
            if rid.starts_with(crate::store::PENDING_MAILBOX) {
                return Err(crate::error::err(ErrorCode::Network, "the folder hasn't been created on the server yet"));
            }
            return Ok(rid);
        }
        Ok(recorded.as_str().context("mailbox remote id")?.to_string())
    }

    /// The messages an op acts on. Their server identity comes from the store
    /// when the op runs: an earlier op for the same message (a move to a
    /// folder, then on to Trash) may have given it a new one since this op was
    /// queued. The recorded memberships stay: they say where it was moved from.
    pub(crate) fn refs_from(&self, payload: &Value, key: &str) -> Result<Vec<MessageRef>> {
        let mut refs: Vec<MessageRef> = serde_json::from_value(payload.get(key).cloned().unwrap_or(json!([])))?;
        for r in &mut refs {
            if let Some(now) = self.store.message_ref(&r.id)? {
                r.remote_id = now.remote_id;
                r.remote_thread_id = now.remote_thread_id;
            }
        }
        Ok(refs)
    }

    pub(crate) fn current_refs(&self, ids: &[String]) -> Result<Vec<MessageRef>> {
        Ok(ids.iter().filter_map(|id| self.store.message_ref(id).ok().flatten()).collect())
    }
}

fn describe(op: &OpRow) -> String {
    match op.kind.as_str() {
        "flags" => "Update flags".into(),
        "move" => "Move messages".into(),
        "labels" => "Change labels".into(),
        "expunge" => "Delete messages".into(),
        "save_draft" => "Save draft".into(),
        "delete_draft" => "Delete draft".into(),
        "send" => format!("Send \"{}\"", op.payload.get("subject").and_then(Value::as_str).unwrap_or("")),
        "cal_put" => "Save event".into(),
        "cal_delete" => "Delete event".into(),
        "contact_put" => "Save contact".into(),
        "contact_delete" => "Delete contact".into(),
        "mailbox_create" => format!("Create folder \"{}\"", op.payload.get("name").and_then(Value::as_str).unwrap_or("")),
        "mailbox_update" => format!("Rename folder \"{}\"", op.payload.get("old_path").and_then(Value::as_str).unwrap_or("")),
        "mailbox_delete" => format!("Delete folder \"{}\"", op.payload.get("path").and_then(Value::as_str).unwrap_or("")),
        k => k.to_string(),
    }
}
