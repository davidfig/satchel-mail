//! The durable op queue: optimistic local changes are recorded as ops and
//! pushed to providers by the account loop, with retry and backoff.

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use satchel_api::*;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::engine::Engine;
use crate::error::is_transient;
use crate::provider::{CalendarOp, Providers, ResolvedOp, SyncCtx};
use crate::store::{MessageRef, OpRow};

const MAX_ATTEMPTS: u32 = 8;

impl Engine {
    pub(crate) fn enqueue(&self, account_id: &str, kind: &str, payload: Value, actor: &str) -> Result<String> {
        let id = self.store.enqueue_op(account_id, kind, &payload, actor)?;
        self.emit(CoreEvent::OutboxChanged);
        self.wake_account(account_id);
        Ok(id)
    }

    pub(crate) async fn run_ops(&self, ctx: &SyncCtx, providers: &Providers) -> Result<()> {
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
                            self.store.delete_cursors_with_prefix(&ctx.account.id, "")?;
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

    fn refs_from(&self, payload: &Value, key: &str) -> Result<Vec<MessageRef>> {
        Ok(serde_json::from_value(payload.get(key).cloned().unwrap_or(json!([])))?)
    }

    fn current_refs(&self, ids: &[String]) -> Result<Vec<MessageRef>> {
        Ok(ids.iter().filter_map(|id| self.store.message_ref(id).ok().flatten()).collect())
    }

    async fn run_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let p = &op.payload;
        let ids = |k: &str| -> Vec<String> { serde_json::from_value(p.get(k).cloned().unwrap_or(json!([]))).unwrap_or_default() };
        if op.kind.starts_with("cal_") {
            return self.run_cal_op(ctx, providers, op).await;
        }
        let mail = providers.mail.as_ref().ok_or_else(|| anyhow!("account has no mail provider"))?;
        let resolved = match op.kind.as_str() {
            "flags" => ResolvedOp::SetFlags {
                messages: self.current_refs(&ids("message_ids"))?,
                seen: p.get("seen").and_then(Value::as_bool),
                flagged: p.get("flagged").and_then(Value::as_bool),
                answered: p.get("answered").and_then(Value::as_bool),
            },
            "move" => ResolvedOp::Move {
                messages: self.refs_from(p, "refs")?,
                to_remote_mailbox: p["to_remote_mailbox"].as_str().context("to_remote_mailbox")?.to_string(),
            },
            "labels" => ResolvedOp::Labels {
                messages: self.refs_from(p, "refs")?,
                add: serde_json::from_value(p["add"].clone())?,
                remove: serde_json::from_value(p["remove"].clone())?,
            },
            "expunge" => ResolvedOp::Expunge { messages: self.refs_from(p, "refs")? },
            "save_draft" => {
                let draft_id = p["draft_id"].as_str().context("draft_id")?;
                let Some((draft, _)) = self.store.get_draft(draft_id)? else { return Ok(()) };
                let raw = self.build_raw(&draft, true)?;
                // The previous server copy (if any) is replaced, not duplicated.
                let replaces = self.server_draft_ref(ctx, draft_id)?;
                let drafts_mb = self.store.mailbox_by_role(&ctx.account.id, MailboxRole::Drafts)?.and_then(|m| self.store.mailbox_remote_id(&m.id).ok().flatten());
                let out = mail.apply(ctx, &ResolvedOp::SaveDraft { raw, replaces: replaces.clone(), drafts_remote_mailbox: drafts_mb }).await?;
                if let Some(old) = replaces.filter(|r| !r.id.is_empty()) {
                    self.store.delete_message(&old.id)?;
                }
                if let Some(rid) = out.stored_remote_id {
                    // The new copy shows up on next sync; remember its remote id.
                    self.store.set_cursor(&ctx.account.id, &format!("draft:{draft_id}"), &rid)?;
                }
                return Ok(());
            }
            "delete_draft" => {
                let rid: Option<String> = p.get("remote_id").and_then(Value::as_str).map(String::from);
                let refs = self.refs_from(p, "refs")?;
                let msg = match (refs.into_iter().next(), rid) {
                    (Some(r), _) => r,
                    (None, Some(rid)) => match self.store.message_id_by_remote(&ctx.account.id, &rid)?.and_then(|id| self.store.message_ref(&id).ok().flatten()) {
                        Some(r) => r,
                        None => return Ok(()),
                    },
                    _ => return Ok(()),
                };
                mail.apply(ctx, &ResolvedOp::DeleteDraft { message: msg.clone() }).await?;
                self.store.delete_message(&msg.id)?;
                return Ok(());
            }
            "send" => {
                let raw = base64::engine::general_purpose::STANDARD.decode(p["raw"].as_str().context("raw")?)?;
                let rcpts: Vec<String> = serde_json::from_value(p["rcpts"].clone())?;
                let from = p["from"].as_str().unwrap_or_default().to_string();
                let sent_mb = self.store.mailbox_by_role(&ctx.account.id, MailboxRole::Sent)?.and_then(|m| self.store.mailbox_remote_id(&m.id).ok().flatten());
                let draft_id = p.get("draft_id").and_then(Value::as_str).map(String::from);
                let server_draft = match &draft_id {
                    Some(d) => self.server_draft_ref(ctx, d)?,
                    None => None,
                };
                let thread_remote_id = p
                    .get("reply_to_message")
                    .and_then(Value::as_str)
                    .and_then(|m| self.store.message_ref(m).ok().flatten())
                    .and_then(|r| r.remote_thread_id);
                mail.apply(ctx, &ResolvedOp::Send { raw, from, rcpts, sent_remote_mailbox: sent_mb, replaces_draft: server_draft.clone(), thread_remote_id }).await?;
                if let Some(d) = draft_id {
                    self.store.delete_draft(&d)?;
                    self.store.delete_cursor(&ctx.account.id, &format!("draft:{d}"))?;
                }
                if let Some(sd) = server_draft.filter(|r| !r.id.is_empty()) {
                    self.store.delete_message(&sd.id)?;
                }
                if let Some(orig) = p.get("reply_to_message").and_then(Value::as_str)
                    && let Some(r) = self.store.message_ref(orig)? {
                        let mut f = r.flags.clone();
                        f.answered = true;
                        self.store.set_flags(orig, &f)?;
                        let _ = mail
                            .apply(ctx, &ResolvedOp::SetFlags { messages: vec![r], seen: None, flagged: None, answered: Some(true) })
                            .await;
                    }
                info!(account = %ctx.account.email, "message sent");
                self.store.audit(&op.actor, "sent", &json!({ "account": ctx.account.email, "subject": p.get("subject") }))?;
                ctx.messages_changed();
                return Ok(());
            }
            other => return Err(anyhow!("unknown op kind {other}")),
        };
        let outcome = mail.apply(ctx, &resolved).await?;
        for (local_id, new_remote, mbs) in outcome.rekeyed {
            let local_mbs: Vec<(String, Option<u32>)> = mbs
                .iter()
                .filter_map(|(r, uid)| ctx.mailbox_local_id(r).ok().flatten().map(|id| (id, *uid)))
                .collect();
            self.store.rekey_message(&local_id, &new_remote, &local_mbs)?;
        }
        ctx.messages_changed();
        Ok(())
    }

    async fn run_cal_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let cal = providers.calendar.as_ref().ok_or_else(|| anyhow!("account has no calendar provider"))?;
        let p = &op.payload;
        match op.kind.as_str() {
            "cal_put" => {
                let event_id = p["event_id"].as_str().context("event_id")?;
                let Some(ev) = self.store.get_event(event_id)? else { return Ok(()) };
                let Some(calendar) = self.store.get_calendar(&ev.calendar_id)? else { return Ok(()) };
                let remote = (!ev.remote_id.starts_with("local:")).then(|| ev.remote_id.clone());
                let out = cal
                    .apply(ctx, &CalendarOp::Put { calendar_remote_id: calendar.remote_id.clone(), event_remote_id: remote.clone(), etag: ev.etag.clone(), uid: ev.uid.clone(), ics: ev.ics.clone() })
                    .await?;
                let rid = out.event_remote_id.or(remote).unwrap_or_else(|| ev.remote_id.clone());
                self.store.set_event_remote(event_id, &rid, out.event_etag.as_deref())?;
            }
            "cal_delete" => {
                let calendar_remote = p["calendar_remote_id"].as_str().context("calendar_remote_id")?.to_string();
                let remote = p["remote_id"].as_str().context("remote_id")?.to_string();
                if !remote.starts_with("local:") {
                    cal.apply(ctx, &CalendarOp::Delete { calendar_remote_id: calendar_remote, event_remote_id: remote, etag: p.get("etag").and_then(Value::as_str).map(String::from) })
                        .await?;
                }
            }
            other => return Err(anyhow!("unknown calendar op {other}")),
        }
        ctx.calendar_changed();
        self.mirror_wake.notify_one();
        Ok(())
    }

    /// The server copy of a local draft, even if it hasn't synced down yet.
    fn server_draft_ref(&self, ctx: &SyncCtx, draft_id: &str) -> Result<Option<MessageRef>> {
        let Some(rid) = self.store.cursor(&ctx.account.id, &format!("draft:{draft_id}"))? else { return Ok(None) };
        if let Some(r) = self.store.message_id_by_remote(&ctx.account.id, &rid)?.and_then(|id| self.store.message_ref(&id).ok().flatten()) {
            return Ok(Some(r));
        }
        Ok(Some(MessageRef {
            id: String::new(),
            account_id: ctx.account.id.clone(),
            remote_id: rid,
            remote_thread_id: None,
            message_id_hdr: None,
            mailboxes: vec![],
            flags: Flags::default(),
            blob_hash: None,
            version: 0,
        }))
    }

    /// Build RFC 822 bytes for a draft.
    pub(crate) fn build_raw(&self, d: &Draft, for_draft_folder: bool) -> Result<Vec<u8>> {
        let acc = self.store.get_account(&d.account_id)?.ok_or_else(|| crate::error::not_found("account"))?;
        let (irt, refs) = match &d.in_reply_to {
            Some(mid) => match self.store.message_row(mid)? {
                Some(m) => {
                    let mut refs = m.references.clone();
                    if let Some(h) = &m.message_id_hdr {
                        refs.push(h.clone());
                    }
                    (m.message_id_hdr.clone(), refs)
                }
                None => (None, vec![]),
            },
            None => (None, vec![]),
        };
        let mut attachments: Vec<(String, String, Vec<u8>)> = vec![];
        for a in &d.attachments {
            attachments.push((a.filename.clone(), a.mime_type.clone(), base64::engine::general_purpose::STANDARD.decode(&a.data_base64)?));
        }
        if let Some(fwd) = &d.forward_of
            && let Some(m) = self.store.message_row(fwd)?
                && let Some(raw) = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten()) {
                    for a in m.attachments.iter().filter(|a| !a.inline) {
                        if let Some((name, mime, data)) = satchel_mime::attachment_bytes(&raw, &a.id) {
                            attachments.push((name, mime, data));
                        }
                    }
                }
        let domain = acc.email.split('@').nth(1).unwrap_or("satchel.local").to_string();
        let mut extra = vec![("X-Mailer".to_string(), "Satchel Mail".to_string())];
        if for_draft_folder
            && let Some(id) = &d.id {
                extra.push(("X-Satchel-Draft-Id".into(), id.clone()));
            }
        let o = satchel_mime::Outgoing {
            from: Address { name: Some(acc.display_name.clone()).filter(|s| !s.is_empty()), email: acc.email.clone() },
            to: d.to.clone(),
            cc: d.cc.clone(),
            bcc: d.bcc.clone(),
            subject: d.subject.clone(),
            text: d.body_text.clone(),
            html: d.body_html.clone(),
            in_reply_to: irt,
            references: refs,
            attachments,
            message_id: satchel_mime::new_message_id(&domain),
            date: chrono::Utc::now(),
            extra_headers: extra,
            include_bcc_header: for_draft_folder,
        };
        Ok(satchel_mime::build(&o))
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
        k => k.to_string(),
    }
}
