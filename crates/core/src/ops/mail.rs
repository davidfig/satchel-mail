//! Mail ops: flags, moves, labels, drafts and sending.

use super::*;

impl Engine {
    pub(crate) async fn run_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let p = &op.payload;
        let ids = |k: &str| -> Vec<String> { serde_json::from_value(p.get(k).cloned().unwrap_or(json!([]))).unwrap_or_default() };
        if op.kind.starts_with("cal_") {
            return self.run_cal_op(ctx, providers, op).await;
        }
        if op.kind.starts_with("contact_") {
            return self.run_contact_op(ctx, providers, op).await;
        }
        if op.kind.starts_with("mailbox_") {
            return self.run_mailbox_op(ctx, providers, op).await;
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
                to_remote_mailbox: self.target_remote(p.get("to_mailbox"), &p["to_remote_mailbox"])?,
            },
            "labels" => {
                // Local ids where recorded: a folder created or renamed since
                // has a different remote id now.
                let lists = |local: &str, remote: &str| -> Result<Vec<String>> {
                    match p.get(local).and_then(Value::as_array) {
                        Some(ids) => ids.iter().map(|i| self.target_remote(Some(i), &Value::Null)).collect(),
                        None => Ok(serde_json::from_value(p[remote].clone())?),
                    }
                };
                ResolvedOp::Labels { messages: self.refs_from(p, "refs")?, add: lists("add_ids", "add")?, remove: lists("remove_ids", "remove")? }
            }
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
                self.store.op_sending(&op.id)?;
                mail.apply(ctx, &ResolvedOp::Send { raw, from, rcpts, sent_remote_mailbox: sent_mb, replaces_draft: server_draft.clone(), thread_remote_id }).await?;
                // Sent: record it before anything else can fail or be cut
                // off, so the op is never replayed. The rest is tidying up.
                self.store.op_done(&op.id)?;
                info!(account = %ctx.account.email, "message sent");
                if let Err(e) = self.store.audit(&op.actor, "sent", &json!({ "account": ctx.account.email, "subject": p.get("subject"), "recipients": p.get("rcpts") })) {
                    warn!("message sent, but recording it in the audit log failed: {e:#}");
                }
                let tidy = async {
                    if let Some(d) = draft_id {
                        self.store.delete_draft(&d)?;
                        self.store.delete_cursor(&ctx.account.id, &format!("draft:{d}"))?;
                    }
                    if let Some(sd) = server_draft.filter(|r| !r.id.is_empty()) {
                        self.store.delete_message(&sd.id)?;
                    }
                    if let Some(orig) = p.get("reply_to_message").and_then(Value::as_str)
                        && let Some(r) = self.store.message_ref(orig)?
                    {
                        let mut f = r.flags.clone();
                        f.answered = true;
                        self.store.set_flags(orig, &f)?;
                        let _ = mail.apply(ctx, &ResolvedOp::SetFlags { messages: vec![r], seen: None, flagged: None, answered: Some(true) }).await;
                    }
                    anyhow::Ok(())
                };
                if let Err(e) = tidy.await {
                    warn!("message sent, but tidying up after it failed: {e:#}");
                }
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
    /// The server copy of a local draft, even if it hasn't synced down yet.
    pub(crate) fn server_draft_ref(&self, ctx: &SyncCtx, draft_id: &str) -> Result<Option<MessageRef>> {
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
            size: 0,
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
                    // Only the account's own mail goes out from it.
                    if m.summary.account_id != d.account_id {
                        return Err(crate::error::invalid("can't forward a message from another account"));
                    }
                    if m.partial && m.attachments.iter().any(|a| !a.inline) {
                        return Err(crate::error::err(ErrorCode::Network, "the forwarded message's attachments aren't downloaded yet; open them, then send again"));
                    }
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
            // SMTP and JMAP deliver to an envelope, so the sent message must not
            // name Bcc recipients. Gmail and Graph send to the recipients named
            // in the headers (and strip Bcc from what others receive), so there
            // Bcc has to be in them.
            include_bcc_header: for_draft_folder || matches!(acc.config, AccountConfig::Gmail { .. } | AccountConfig::Graph { .. }),
        };
        Ok(satchel_mime::build(&o))
    }
}
