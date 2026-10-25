//! Applying ops through labels, drafts and messages.send.

use super::*;

impl GmailProvider {
    pub(super) async fn apply_op(&self, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        let ids = |m: &[MessageRef]| m.iter().map(|x| x.remote_id.clone()).collect::<Vec<_>>();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, .. } => {
                let mut add = vec![];
                let mut remove = vec![];
                match seen {
                    Some(true) => remove.push("UNREAD".to_string()),
                    Some(false) => add.push("UNREAD".to_string()),
                    None => {}
                }
                match flagged {
                    Some(true) => add.push("STARRED".to_string()),
                    Some(false) => remove.push("STARRED".to_string()),
                    None => {}
                }
                if !add.is_empty() || !remove.is_empty() {
                    self.modify(ids(messages), add, remove).await?;
                }
            }
            ResolvedOp::Labels { messages, add, remove } => {
                let add: Vec<String> = add.iter().filter(|a| *a != "ALL").cloned().collect();
                let remove: Vec<String> = remove.iter().filter(|a| *a != "ALL").cloned().collect();
                self.modify(ids(messages), add, remove).await?;
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                self.modify(ids(messages), vec![to_remote_mailbox.clone()], vec!["INBOX".into()]).await?;
            }
            ResolvedOp::Expunge { messages } => {
                self.post(&format!("{GMAIL}/messages/batchDelete"), json!({ "ids": ids(messages) }), "delete messages").await?;
            }
            ResolvedOp::SaveDraft { raw, replaces, .. } => {
                let body = json!({ "message": { "raw": b64url(raw) } });
                let existing = match replaces {
                    Some(r) => self.draft_id_for(&r.remote_id).await?,
                    None => None,
                };
                let v = match existing {
                    Some(d) => self.api.json(|h, t| h.put(format!("{GMAIL}/drafts/{d}")).bearer_auth(t).json(&body), "update draft").await?,
                    None => self.post(&format!("{GMAIL}/drafts"), body.clone(), "create draft").await?,
                };
                out.stored_remote_id = v["message"]["id"].as_str().map(String::from);
            }
            ResolvedOp::DeleteDraft { message } => {
                if let Some(d) = self.draft_id_for(&message.remote_id).await? {
                    self.api.send(|h, t| h.delete(format!("{GMAIL}/drafts/{d}")).bearer_auth(t), "delete draft").await?;
                }
            }
            // Gmail nests labels by name ("Parent/Child"), so the label's name is its full path.
            ResolvedOp::CreateMailbox { path, .. } => {
                let body = json!({ "name": path, "labelListVisibility": "labelShow", "messageListVisibility": "show" });
                let v = self.post(&format!("{GMAIL}/labels"), body, "create label").await?;
                out.mailbox_remote_id = Some(v["id"].as_str().context("Gmail returned no label id")?.to_string());
            }
            ResolvedOp::UpdateMailbox { remote_id, old_path, path, .. } => {
                // Nested labels are separate labels named after their parent: rename them too.
                let all = self.get(&format!("{GMAIL}/labels"), "list labels").await?;
                let under = format!("{old_path}/");
                let mut renames = vec![(remote_id.clone(), path.clone())];
                for l in all["labels"].as_array().cloned().unwrap_or_default() {
                    if let (Some(id), Some(name)) = (l["id"].as_str(), l["name"].as_str())
                        && let Some(rest) = name.strip_prefix(&under)
                    {
                        renames.push((id.to_string(), format!("{path}/{rest}")));
                    }
                }
                for (id, name) in renames {
                    let body = json!({ "name": name });
                    self.api.json(|h, t| h.patch(format!("{GMAIL}/labels/{id}")).bearer_auth(t).json(&body), "rename label").await?;
                }
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => {
                // Removes the label from its messages; the messages stay.
                self.api.send(|h, t| h.delete(format!("{GMAIL}/labels/{remote_id}")).bearer_auth(t), "delete label").await?;
            }
            ResolvedOp::Send { raw, replaces_draft, thread_remote_id, .. } => {
                let mut body = json!({ "raw": b64url(raw) });
                if let Some(t) = thread_remote_id {
                    body["threadId"] = json!(t);
                }
                self.post(&format!("{GMAIL}/messages/send"), body, "send").await.map_err(crate::net::send_error)?;
                if let Some(d) = replaces_draft
                    && let Ok(Some(did)) = self.draft_id_for(&d.remote_id).await {
                        let _ = self.api.send(|h, t| h.delete(format!("{GMAIL}/drafts/{did}")).bearer_auth(t), "delete draft").await;
                    }
            }
        }
        Ok(out)
    }
}
