//! Outlook mail: folders, delta sync per folder, and sending.

use super::*;

const SELECT: &str = "id,conversationId,internetMessageId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,isRead,isDraft,flag,hasAttachments,bodyPreview,parentFolderId";
const WELL_KNOWN: [(&str, MailboxRole); 6] = [
    ("inbox", MailboxRole::Inbox),
    ("sentitems", MailboxRole::Sent),
    ("drafts", MailboxRole::Drafts),
    ("deleteditems", MailboxRole::Trash),
    ("junkemail", MailboxRole::Junk),
    ("archive", MailboxRole::Archive),
];
pub fn record(m: &Value) -> MessageRecord {
    let date = m["receivedDateTime"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or_else(Utc::now);
    MessageRecord {
        remote_id: m["id"].as_str().unwrap_or_default().to_string(),
        remote_thread_id: m["conversationId"].as_str().map(String::from),
        message_id_hdr: m["internetMessageId"].as_str().map(|s| s.trim_matches(['<', '>']).to_string()),
        in_reply_to: None,
        references: vec![],
        subject: m["subject"].as_str().unwrap_or_default().to_string(),
        from: addr(&m["from"]),
        to: addrs(&m["toRecipients"]),
        cc: addrs(&m["ccRecipients"]),
        bcc: addrs(&m["bccRecipients"]),
        reply_to: addrs(&m["replyTo"]),
        date,
        snippet: m["bodyPreview"].as_str().unwrap_or_default().chars().take(200).collect(),
        flags: Flags {
            seen: m["isRead"] == true,
            flagged: m["flag"]["flagStatus"] == "flagged",
            answered: false,
            draft: m["isDraft"] == true,
        },
        attachments: vec![],
        blob_hash: None,
        size: 0,
    }
}

pub struct GraphMailProvider {
    api: ApiClient,
}

impl GraphMailProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: api_client(creds)? })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        self.api.json(|h, t| with_prefs(h.get(url)).bearer_auth(t), what).await
    }

    async fn folders(&self) -> Result<Vec<RemoteMailbox>> {
        let mut roles: HashMap<String, MailboxRole> = HashMap::new();
        for (wk, role) in WELL_KNOWN {
            if let Ok(v) = self.get(&format!("{ME}/mailFolders/{wk}?$select=id"), "well-known folder").await
                && let Some(id) = v["id"].as_str() {
                    roles.insert(id.to_string(), role);
                }
        }
        let mut out = vec![];
        let mut queue: Vec<(String, String)> = vec![(format!("{ME}/mailFolders?$top=250&$select=id,displayName,childFolderCount"), String::new())];
        while let Some((url, prefix)) = queue.pop() {
            let mut next = Some(url);
            while let Some(u) = next {
                let v = self.get(&u, "list folders").await?;
                for f in v["value"].as_array().cloned().unwrap_or_default() {
                    let id = f["id"].as_str().unwrap_or_default().to_string();
                    let name = f["displayName"].as_str().unwrap_or_default().to_string();
                    let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    if f["childFolderCount"].as_u64().unwrap_or(0) > 0 {
                        queue.push((format!("{ME}/mailFolders/{id}/childFolders?$top=250&$select=id,displayName,childFolderCount"), path.clone()));
                    }
                    let role = roles.get(&id).copied().unwrap_or(MailboxRole::Custom);
                    out.push(RemoteMailbox { remote_id: id, name: if role == MailboxRole::Inbox { "Inbox".into() } else { name }, path, role, is_label: false });
                }
                next = v["@odata.nextLink"].as_str().map(String::from);
            }
        }
        // Hide Outlook system folders that aren't mail.
        out.retain(|m| !matches!(m.path.as_str(), "Conversation History" | "Sync Issues" | "Outbox" | "RSS Feeds" | "RSS Subscriptions"));
        Ok(out)
    }

    async fn sync_folder(&self, ctx: &SyncCtx, folder: &RemoteMailbox) -> Result<()> {
        let key = format!("graph:delta:{}", folder.remote_id);
        let mut url = ctx.cursor(&key)?.unwrap_or_else(|| format!("{ME}/mailFolders/{}/messages/delta?$select={SELECT}", folder.remote_id));
        let local_mb = ctx.mailbox_local_id(&folder.remote_id)?;
        let mut n = 0;
        loop {
            let v = match self.get(&url, "message delta").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 410") || e.to_string().contains("syncStateNotFound") => {
                    ctx.store.delete_cursor(ctx.account_id(), &key)?;
                    url = format!("{ME}/mailFolders/{}/messages/delta?$select={SELECT}", folder.remote_id);
                    continue;
                }
                Err(e) => return Err(e),
            };
            for m in v["value"].as_array().cloned().unwrap_or_default() {
                let id = m["id"].as_str().unwrap_or_default();
                if m.get("@removed").is_some() {
                    if let (Some(local), Some(mb)) = (ctx.message_by_remote(id)?, local_mb.as_ref()) {
                        ctx.store.remove_from_mailbox(&local, mb)?;
                    }
                    continue;
                }
                let parent = m["parentFolderId"].as_str().unwrap_or(&folder.remote_id).to_string();
                ctx.put_record(record(&m), &[(parent, None)])?;
                n += 1;
                if n % 200 == 0 {
                    ctx.progress(format!("{}: {n} messages", folder.name));
                }
            }
            if let Some(next) = v["@odata.nextLink"].as_str() {
                url = next.to_string();
                continue;
            }
            if let Some(delta) = v["@odata.deltaLink"].as_str() {
                ctx.set_cursor(&key, delta)?;
            }
            break;
        }
        Ok(())
    }

    async fn mime_upload(&self, url: &str, raw: &[u8], what: &str) -> Result<Value> {
        let body = base64::engine::general_purpose::STANDARD.encode(raw);
        self.api.json(|h, t| with_prefs(h.post(url)).bearer_auth(t).header("Content-Type", "text/plain").body(body.clone()), what).await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        match self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/messages/{id}"))).bearer_auth(t), "delete message").await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("HTTP 404") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl MailProvider for GraphMailProvider {
    async fn test(&self) -> Result<()> {
        self.get(&format!("{ME}?$select=id"), "profile").await?;
        Ok(())
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let folders = self.folders().await?;
        ctx.set_mailboxes(&folders)?;
        let mut order: Vec<&RemoteMailbox> = folders.iter().collect();
        order.sort_by_key(|f| if f.role == MailboxRole::Inbox { 0 } else { 1 });
        for f in order {
            if let Err(e) = self.sync_folder(ctx, f).await {
                warn!(folder = %f.path, "Graph sync: {e:#}");
                return Err(e);
            }
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let r = self.api.send(|h, t| with_prefs(h.get(format!("{ME}/messages/{}/$value", msg.remote_id))).bearer_auth(t), "get MIME").await?;
        Ok(r.bytes().await?.to_vec())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, .. } => {
                let mut patch = serde_json::Map::new();
                if let Some(s) = seen {
                    patch.insert("isRead".into(), json!(s));
                }
                if let Some(f) = flagged {
                    patch.insert("flag".into(), json!({ "flagStatus": if *f { "flagged" } else { "notFlagged" } }));
                }
                if !patch.is_empty() {
                    let body = Value::Object(patch);
                    let urls: Vec<String> = messages.iter().map(|m| format!("{ME}/messages/{}", m.remote_id)).collect();
                    let results: Vec<Result<Value>> = futures::stream::iter(urls.into_iter().map(|url| {
                        let body = body.clone();
                        async move { self.api.json(|h, t| with_prefs(h.patch(&url)).bearer_auth(t).json(&body), "update message").await }
                    }))
                    .buffer_unordered(4)
                    .collect()
                    .await;
                    for r in results {
                        r?;
                    }
                }
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                for m in messages {
                    let v = self
                        .api
                        .json(|h, t| with_prefs(h.post(format!("{ME}/messages/{}/move", m.remote_id))).bearer_auth(t).json(&json!({ "destinationId": to_remote_mailbox })), "move message")
                        .await?;
                    let new_id = v["id"].as_str().unwrap_or(&m.remote_id).to_string();
                    out.rekeyed.push((m.id.clone(), new_id, vec![(to_remote_mailbox.clone(), None)]));
                }
            }
            ResolvedOp::Labels { .. } => anyhow::bail!("Outlook accounts use folders, not labels"),
            ResolvedOp::Expunge { messages } => {
                for m in messages {
                    self.delete(&m.remote_id).await?;
                }
            }
            ResolvedOp::SaveDraft { raw, replaces, .. } => {
                let v = self.mime_upload(&format!("{ME}/messages"), raw, "create draft").await?;
                out.stored_remote_id = v["id"].as_str().map(String::from);
                if let Some(old) = replaces {
                    let _ = self.delete(&old.remote_id).await;
                }
            }
            ResolvedOp::DeleteDraft { message } => self.delete(&message.remote_id).await?,
            ResolvedOp::CreateMailbox { name, parent, .. } => {
                let url = match parent {
                    Some(p) => format!("{ME}/mailFolders/{p}/childFolders"),
                    None => format!("{ME}/mailFolders"),
                };
                let body = json!({ "displayName": name });
                let v: Value = self.api.json(|h, t| with_prefs(h.post(&url)).bearer_auth(t).json(&body), "create folder").await?;
                out.mailbox_remote_id = Some(v["id"].as_str().context("Graph returned no folder id")?.to_string());
            }
            ResolvedOp::UpdateMailbox { remote_id, old_path, name, parent, path } => {
                let parent_of = |p: &str| p.rsplit_once('/').map(|(a, _)| a.to_string());
                if parent_of(old_path) != parent_of(path) {
                    let body = json!({ "destinationId": parent.as_deref().unwrap_or("msgfolderroot") });
                    self.api.json(|h, t| with_prefs(h.post(format!("{ME}/mailFolders/{remote_id}/move"))).bearer_auth(t).json(&body), "move folder").await?;
                }
                if old_path.rsplit('/').next() != Some(name.as_str()) {
                    let body = json!({ "displayName": name });
                    self.api.json(|h, t| with_prefs(h.patch(format!("{ME}/mailFolders/{remote_id}"))).bearer_auth(t).json(&body), "rename folder").await?;
                }
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => {
                // The core moves the messages out first; never delete a folder that still has any.
                let f = self.get(&format!("{ME}/mailFolders/{remote_id}?$select=totalItemCount"), "get folder").await?;
                if f["totalItemCount"].as_u64().unwrap_or(0) > 0 {
                    return Err(ApiError::new(ErrorCode::Conflict, "not deleted: the folder still has messages on the server").into());
                }
                self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/mailFolders/{remote_id}"))).bearer_auth(t), "delete folder").await?;
            }
            ResolvedOp::Send { raw, replaces_draft, .. } => {
                self.mime_upload(&format!("{ME}/sendMail"), raw, "send").await.map_err(crate::net::send_error)?;
                if let Some(d) = replaces_draft {
                    let _ = self.delete(&d.remote_id).await;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_record() {
        let m = json!({ "id": "AAMk", "conversationId": "C1", "internetMessageId": "<x@y>", "subject": "Hi",
            "from": { "emailAddress": { "name": "Bob", "address": "bob@x.test" } }, "toRecipients": [{ "emailAddress": { "address": "a@x.test" } }],
            "receivedDateTime": "2026-02-03T04:05:06Z", "isRead": false, "flag": { "flagStatus": "flagged" }, "bodyPreview": "hello" });
        let r = record(&m);
        assert_eq!(r.message_id_hdr.as_deref(), Some("x@y"));
        assert!(!r.flags.seen && r.flags.flagged);
        assert_eq!(r.to[0].email, "a@x.test");
    }
}
