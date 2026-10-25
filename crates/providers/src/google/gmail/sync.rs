//! Syncing: the recent window, history deltas and the backfill of older mail.

use super::*;

impl GmailProvider {
    pub(super) async fn fetch_metadata(&self, ctx: &SyncCtx, ids: Vec<String>, known_labels: &HashSet<String>) -> Result<()> {
        let q: String = META_HEADERS.iter().map(|h| format!("&metadataHeaders={h}")).collect();
        let total = ids.len();
        let mut done = 0;
        let mut stream = futures::stream::iter(ids.into_iter().map(|id| {
            let url = format!("{GMAIL}/messages/{id}?format=metadata{q}");
            async move { (id, self.get(&url, "get message").await) }
        }))
        .buffer_unordered(CONCURRENCY);
        while let Some((id, r)) = stream.next().await {
            done += 1;
            if done % 100 == 0 {
                ctx.progress(format!("Downloading headers {done}/{total}"));
            }
            match r {
                Ok(m) => {
                    let mbs = memberships(&label_ids(&m), known_labels);
                    ctx.put_record(record_from_metadata(&m), &mbs)?;
                }
                Err(e) if e.to_string().contains("404") => ctx.delete_remote(&id)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Update messages we have from the server's current labels (`minimal`
    /// is just ids and labels); delete the ones it no longer has.
    pub(super) async fn refresh_known(&self, ctx: &SyncCtx, ids: Vec<String>, known_labels: &HashSet<String>) -> Result<()> {
        let total = ids.len();
        let mut done = 0;
        let mut stream = futures::stream::iter(ids.into_iter().map(|id| {
            let url = format!("{GMAIL}/messages/{id}?format=minimal");
            async move { (id, self.get(&url, "get message").await) }
        }))
        .buffer_unordered(CONCURRENCY);
        while let Some((id, r)) = stream.next().await {
            done += 1;
            if done % 100 == 0 {
                ctx.progress(format!("Checking messages {done}/{total}"));
            }
            match r {
                Ok(m) => {
                    let labs = label_ids(&m);
                    ctx.set_flags_remote(&id, flags_from_labels(&labs))?;
                    ctx.set_memberships_remote(&id, &memberships(&labs, known_labels))?;
                }
                Err(e) if e.to_string().contains("HTTP 404") => ctx.delete_remote(&id)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub(super) async fn list_ids(&self, q: &str, page: Option<String>, max: usize) -> Result<(Vec<String>, Option<String>)> {
        let mut ids = vec![];
        let mut token = page;
        loop {
            let mut url = format!("{GMAIL}/messages?maxResults=500&includeSpamTrash=true&q={}", crate::net::enc(q));
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = self.get(&url, "list messages").await?;
            ids.extend(v["messages"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|m| m["id"].as_str().map(String::from)));
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() || ids.len() >= max {
                return Ok((ids, token));
            }
        }
    }

    /// The recent window from scratch: new messages are downloaded, and ones
    /// we have are brought up to date (labels, read and starred) or removed
    /// if the server deleted them. Runs on the first sync and when the
    /// history id has expired, when changes since are unknown.
    pub(super) async fn full_sync(&self, ctx: &SyncCtx, labels: &HashSet<String>) -> Result<()> {
        let profile = self.get(&format!("{GMAIL}/profile"), "profile").await?;
        let history = profile["historyId"].as_str().context("no historyId")?.to_string();
        let days = ctx.settings.initial_sync_days.max(7);
        let (ids, _) = self.list_ids(&format!("newer_than:{days}d"), None, usize::MAX).await?;
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let missing: Vec<String> = ids.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        self.fetch_metadata(ctx, missing, labels).await?;
        // Known ones in the window, plus ones dated in it the server no longer
        // lists (deleted, most likely; the check confirms it).
        let mut refresh: HashSet<String> = ids.iter().filter(|i| known.contains_key(*i)).cloned().collect();
        refresh.extend(ctx.store.remote_ids_since(ctx.account_id(), Utc::now() - chrono::Duration::days(days as i64))?);
        self.refresh_known(ctx, refresh.into_iter().collect(), labels).await?;
        ctx.set_cursor("gmail:history", &history)?;
        ctx.set_cursor("gmail:backfill", "start")?;
        Ok(())
    }

    /// Older mail, a page at a time across syncs.
    pub(super) async fn backfill(&self, ctx: &SyncCtx, labels: &HashSet<String>) -> Result<()> {
        let Some(state) = ctx.cursor("gmail:backfill")? else { return Ok(()) };
        if state == "done" {
            return Ok(());
        }
        let days = ctx.settings.initial_sync_days.max(7);
        let page = (state != "start").then_some(state);
        let (ids, next) = self.list_ids(&format!("older_than:{days}d"), page, 1000).await?;
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let missing: Vec<String> = ids.into_iter().filter(|i| !known.contains_key(i)).collect();
        self.fetch_metadata(ctx, missing, labels).await?;
        ctx.set_cursor("gmail:backfill", next.as_deref().unwrap_or("done"))?;
        Ok(())
    }

    pub(super) async fn delta(&self, ctx: &SyncCtx, start: &str, labels: &HashSet<String>) -> Result<bool> {
        let mut token: Option<String> = None;
        let mut added: HashSet<String> = HashSet::new();
        let mut relabeled: HashMap<String, Vec<String>> = HashMap::new();
        let mut deleted: HashSet<String> = HashSet::new();
        let mut latest = start.to_string();
        loop {
            let mut url = format!("{GMAIL}/history?startHistoryId={start}&maxResults=500");
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = match self.get(&url, "history").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 404") => return Ok(false),
                Err(e) => return Err(e),
            };
            for h in v["history"].as_array().cloned().unwrap_or_default() {
                for a in h["messagesAdded"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = a["message"]["id"].as_str() {
                        added.insert(id.to_string());
                        deleted.remove(id);
                    }
                }
                for d in h["messagesDeleted"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = d["message"]["id"].as_str() {
                        deleted.insert(id.to_string());
                        added.remove(id);
                        relabeled.remove(id);
                    }
                }
                for key in ["labelsAdded", "labelsRemoved"] {
                    for l in h[key].as_array().cloned().unwrap_or_default() {
                        if let Some(id) = l["message"]["id"].as_str() {
                            relabeled.insert(id.to_string(), label_ids(&l["message"]));
                        }
                    }
                }
            }
            if let Some(hid) = v["historyId"].as_str() {
                latest = hid.to_string();
            }
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() {
                break;
            }
        }
        for id in &deleted {
            ctx.delete_remote(id)?;
        }
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let new: Vec<String> = added.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        self.fetch_metadata(ctx, new, labels).await?;
        for (id, labs) in relabeled {
            if added.contains(&id) && !known.contains_key(&id) {
                continue;
            }
            ctx.set_flags_remote(&id, flags_from_labels(&labs))?;
            ctx.set_memberships_remote(&id, &memberships(&labs, labels))?;
        }
        ctx.set_cursor("gmail:history", &latest)?;
        Ok(true)
    }
}
