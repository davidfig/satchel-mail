//! Outlook mail: folders, delta sync per folder, and sending.

use super::*;

const SELECT: &str = "id,conversationId,internetMessageId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,isRead,isDraft,flag,hasAttachments,bodyPreview,parentFolderId";
/// Pages of older mail (200 messages each) fetched from a folder before
/// moving on to the next.
const PAGES_PER_TURN: usize = 5;
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
        ..Default::default()
    }
}

pub struct GraphMailProvider {
    api: ApiClient,
    /// Folders the last sync left with older mail to fetch, for
    /// [`MailProvider::backfill`].
    behind: std::sync::Mutex<Vec<RemoteMailbox>>,
    /// For those folders, the newest receivedDateTime fetched by `newest`, so
    /// each pass asks only for mail after it.
    newest_seen: std::sync::Mutex<HashMap<String, String>>,
}

impl GraphMailProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: api_client(creds)?, behind: Default::default(), newest_seen: Default::default() })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        graph_link(url)?;
        self.api.json(|h, t| with_prefs(h.get(url)).bearer_auth(t), what).await
    }

    async fn folders(&self) -> Result<Vec<RemoteMailbox>> {
        let mut roles: HashMap<String, MailboxRole> = HashMap::new();
        for (wk, role) in WELL_KNOWN {
            if let Ok(v) = self.get(&format!("{ME}/mailFolders/{}?$select=id", crate::net::enc(wk)), "well-known folder").await
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
                        queue.push((format!("{ME}/mailFolders/{}/childFolders?$top=250&$select=id,displayName,childFolderCount", crate::net::enc(&id)), path.clone()));
                    }
                    let role = roles.get(&id).copied().unwrap_or(MailboxRole::Custom);
                    out.push(RemoteMailbox { remote_id: id, name: if role == MailboxRole::Inbox { "Inbox".into() } else { name }, path, role, is_label: false });
                }
                next = v["@odata.nextLink"].as_str().map(graph_link).transpose()?;
            }
        }
        // Hide Outlook system folders that aren't mail.
        out.retain(|m| !matches!(m.path.as_str(), "Conversation History" | "Sync Issues" | "Outbox" | "RSS Feeds" | "RSS Subscriptions"));
        Ok(out)
    }

    /// One folder's delta, resuming where the last call stopped. The first
    /// round comes newest first, so stopping early leaves only older mail
    /// for later. Stops after `max_pages`, or once a page reaches mail
    /// received before `stop_before`. Returns whether the folder is caught up.
    async fn sync_folder(&self, ctx: &SyncCtx, folder: &RemoteMailbox, stop_before: Option<DateTime<Utc>>, max_pages: usize) -> Result<bool> {
        let key = format!("graph:delta:{}", folder.remote_id);
        let start = format!("{ME}/mailFolders/{}/messages/delta?$select={SELECT}&$orderby=receivedDateTime%20desc", crate::net::enc(&folder.remote_id));
        // A saved cursor that isn't a Graph URL (a bad link stored before we
        // checked) is dropped, and the folder synced from the start.
        let mut url = ctx.cursor(&key)?.filter(|u| is_graph(u)).unwrap_or_else(|| start.clone());
        let local_mb = ctx.mailbox_local_id(&folder.remote_id)?;
        let mut n = 0;
        for _ in 0..max_pages {
            let v = match self.get(&url, "message delta").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 410") || e.to_string().contains("syncStateNotFound") => {
                    ctx.store.delete_cursor(ctx.account_id(), &key)?;
                    url = start.clone();
                    continue;
                }
                Err(e) => return Err(e),
            };
            let mut oldest: Option<DateTime<Utc>> = None;
            for m in v["value"].as_array().cloned().unwrap_or_default() {
                let id = m["id"].as_str().unwrap_or_default();
                if m.get("@removed").is_some() {
                    if let (Some(local), Some(mb)) = (ctx.message_by_remote(id)?, local_mb.as_ref()) {
                        ctx.store.remove_from_mailbox(&local, mb)?;
                    }
                    continue;
                }
                let parent = m["parentFolderId"].as_str().unwrap_or(&folder.remote_id).to_string();
                let rec = record(&m);
                oldest = Some(oldest.map_or(rec.date, |o| o.min(rec.date)));
                ctx.put_record(rec, &[(parent, None)])?;
                n += 1;
                if n % 200 == 0 {
                    ctx.progress(format!("{}: {n} messages", folder.name));
                }
            }
            if let Some(delta) = v["@odata.deltaLink"].as_str() {
                ctx.set_cursor(&key, &graph_link(delta)?)?;
                return Ok(true);
            }
            let Some(next) = v["@odata.nextLink"].as_str() else { return Ok(true) };
            let next = graph_link(next)?;
            // Saved so the round resumes here, in this pass or a later one.
            ctx.set_cursor(&key, &next)?;
            url = next;
            if stop_before.is_some_and(|s| oldest.is_some_and(|o| o < s)) {
                break;
            }
        }
        Ok(false)
    }

    /// New messages in a folder whose delta round is still going. Mail that
    /// arrived after the round began only comes with the next round, so
    /// without this it would wait for the whole backfill (0047). The first
    /// call takes the newest 50; later ones only what arrived since.
    async fn newest(&self, ctx: &SyncCtx, folder: &RemoteMailbox) -> Result<()> {
        let since = self.newest_seen.lock().unwrap().get(&folder.remote_id).cloned();
        // `ge`, not `gt`: two messages can share a second.
        let filter = since.as_deref().map(|t| format!("&$filter=receivedDateTime%20ge%20{}", crate::net::enc(t))).unwrap_or_default();
        let url = format!("{ME}/mailFolders/{}/messages?$select={SELECT}&$orderby=receivedDateTime%20desc&$top=50{filter}", crate::net::enc(&folder.remote_id));
        let v = self.get(&url, "newest messages").await?;
        let found = v["value"].as_array().cloned().unwrap_or_default();
        for m in &found {
            let parent = m["parentFolderId"].as_str().unwrap_or(&folder.remote_id).to_string();
            ctx.put_record(record(m), &[(parent, None)])?;
        }
        if let Some(t) = found.first().and_then(|m| m["receivedDateTime"].as_str()) {
            self.newest_seen.lock().unwrap().insert(folder.remote_id.clone(), t.to_string());
        }
        Ok(())
    }

    async fn mime_upload(&self, url: &str, raw: &[u8], what: &str) -> Result<Value> {
        let body = base64::engine::general_purpose::STANDARD.encode(raw);
        self.api.json(|h, t| with_prefs(h.post(url)).bearer_auth(t).header("Content-Type", "text/plain").body(body.clone()), what).await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        match self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/messages/{}", crate::net::enc(id)))).bearer_auth(t), "delete message").await {
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
        // Recent mail in every folder; older mail follows in `backfill` (0047).
        let window = Utc::now() - chrono::Duration::days(ctx.settings.initial_sync_days.max(7) as i64);
        let mut behind = vec![];
        for f in order {
            match self.sync_folder(ctx, f, Some(window), usize::MAX).await {
                Ok(true) => {
                    self.newest_seen.lock().unwrap().remove(&f.remote_id);
                }
                Ok(false) => {
                    if let Err(e) = self.newest(ctx, f).await {
                        warn!(folder = %f.path, "Graph newest mail: {e:#}");
                    }
                    behind.push(f.clone());
                }
                Err(e) => {
                    warn!(folder = %f.path, "Graph sync: {e:#}");
                    return Err(e);
                }
            }
        }
        *self.behind.lock().unwrap() = behind;
        Ok(())
    }

    /// A few pages per folder in turn, so they all move back in time together.
    async fn backfill(&self, ctx: &SyncCtx, until: std::time::Instant) -> Result<()> {
        let mut behind = std::mem::take(&mut *self.behind.lock().unwrap());
        while !behind.is_empty() {
            let mut still = vec![];
            for f in behind {
                if std::time::Instant::now() >= until || !self.sync_folder(ctx, &f, None, PAGES_PER_TURN).await? {
                    still.push(f);
                }
            }
            behind = still;
            if std::time::Instant::now() >= until {
                break;
            }
        }
        if !behind.is_empty() {
            ctx.more_to_sync();
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let r = self.api.send(|h, t| with_prefs(h.get(format!("{ME}/messages/{}/$value", crate::net::enc(&msg.remote_id)))).bearer_auth(t), "get MIME").await?;
        crate::net::body_capped(r, crate::net::DOWNLOAD_LIMIT).await
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
                    let urls: Vec<String> = messages.iter().map(|m| format!("{ME}/messages/{}", crate::net::enc(&m.remote_id))).collect();
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
                        .json(|h, t| with_prefs(h.post(format!("{ME}/messages/{}/move", crate::net::enc(&m.remote_id)))).bearer_auth(t).json(&json!({ "destinationId": to_remote_mailbox })), "move message")
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
                    Some(p) => format!("{ME}/mailFolders/{}/childFolders", crate::net::enc(p)),
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
                    self.api.json(|h, t| with_prefs(h.post(format!("{ME}/mailFolders/{}/move", crate::net::enc(remote_id)))).bearer_auth(t).json(&body), "move folder").await?;
                }
                if old_path.rsplit('/').next() != Some(name.as_str()) {
                    let body = json!({ "displayName": name });
                    self.api.json(|h, t| with_prefs(h.patch(format!("{ME}/mailFolders/{}", crate::net::enc(remote_id)))).bearer_auth(t).json(&body), "rename folder").await?;
                }
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => {
                // The core moves the messages out first; never delete a folder that still has any.
                let f = self.get(&format!("{ME}/mailFolders/{}?$select=totalItemCount", crate::net::enc(remote_id)), "get folder").await?;
                if f["totalItemCount"].as_u64().unwrap_or(0) > 0 {
                    return Err(ApiError::new(ErrorCode::Conflict, "not deleted: the folder still has messages on the server").into());
                }
                self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/mailFolders/{}", crate::net::enc(remote_id)))).bearer_auth(t), "delete folder").await?;
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
