//! Drafts, replies and sending.


use anyhow::Result;
use base64::Engine as _;
use satchel_api::*;
use serde_json::json;

use crate::engine::{Engine, Mode};
use crate::error::{forbidden, invalid, not_found};
use crate::store::new_id;

impl Engine {
    pub(crate) async fn compose_reply(&self, message_id: &str, mode: ReplyMode) -> Result<Draft> {
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
    pub(crate) fn save_draft(&self, mut draft: Draft, sync_to_server: bool) -> Result<String> {
        self.store.get_account(&draft.account_id)?.ok_or_else(|| not_found("account"))?;
        check_addresses(&draft, false)?;
        if draft.id.is_none() {
            draft.id = Some(new_id());
        }
        let id = self.store.save_draft(&draft)?;
        if sync_to_server && self.mode == Mode::Full {
            self.enqueue(&draft.account_id, "save_draft", json!({ "draft_id": id }), "user")?;
        }
        Ok(id)
    }

    pub(crate) async fn edit_draft(&self, message_id: &str) -> Result<Draft> {
        let m = self.store.message_row(message_id)?.ok_or_else(|| not_found("message"))?;
        let account = m.summary.account_id.clone();
        let raw = self.raw_for(message_id).await?.ok_or_else(|| not_found("message body"))?;
        // Satchel's own drafts name their local draft.
        if let Some(id) = header_value(&raw, "X-Satchel-Draft-Id")
            && let Some((d, _)) = self.store.get_draft(&id)?
        {
            return Ok(d);
        }
        let p = satchel_mime::parse(&raw).unwrap_or_default();
        let mut attachments = vec![];
        for a in p.attachments.iter().filter(|a| !a.inline) {
            if let Some((filename, mime_type, data)) = satchel_mime::attachment_bytes(&raw, &a.id) {
                attachments.push(OutgoingAttachment { filename, mime_type, data_base64: base64::engine::general_purpose::STANDARD.encode(data) });
            }
        }
        let in_reply_to = match &p.in_reply_to {
            Some(h) => self.store.message_id_by_header(&account, h)?,
            None => None,
        };
        let draft = Draft {
            id: None,
            account_id: account.clone(),
            to: p.to,
            cc: p.cc,
            bcc: p.bcc,
            subject: p.subject,
            body_text: p.text,
            body_html: None,
            in_reply_to,
            forward_of: None,
            attachments,
        };
        let id = self.store.save_draft(&draft)?;
        // Saving or sending it replaces this server copy rather than adding one.
        self.store.set_cursor(&account, &format!("draft:{id}"), &m.remote_id)?;
        Ok(self.store.get_draft(&id)?.map(|(d, _)| d).unwrap_or(draft))
    }

    pub(crate) fn delete_draft(&self, id: &str) -> Result<()> {
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
        check_addresses(&draft, true)?;
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
}

/// One header's value from a raw message, unfolded.
pub(crate) fn header_value(raw: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let unfolded = head.replace("\r\n ", " ").replace("\r\n\t", " ");
    unfolded.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// Recipients must be plain addresses. Addresses go into headers as written,
/// so one carrying a line break or angle brackets could add headers of its
/// own (a hidden Bcc, from an agent's proposal). `complete` also requires a
/// whole address; a draft being typed may hold half of one.
pub(crate) fn check_addresses(d: &Draft, complete: bool) -> Result<()> {
    for a in d.to.iter().chain(&d.cc).chain(&d.bcc) {
        let e = a.email.as_str();
        let safe = !e.chars().any(|c| c.is_control() || "<>\"\\".contains(c));
        let whole = e.split_once('@').is_some_and(|(l, h)| !l.is_empty() && !h.is_empty()) && !e.chars().any(|c| c.is_whitespace() || ",;:()[]".contains(c));
        if !safe || (complete && !whole) {
            return Err(invalid(format!("invalid address: {}", e.escape_debug())));
        }
    }
    Ok(())
}
