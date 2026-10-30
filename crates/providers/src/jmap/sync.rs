//! Full and delta sync of Email objects.

use super::*;

/// Email/query pages (1000 ids each) listed in one full sync pass.
const LIST_PAGES_PER_PASS: usize = 100;
/// Email/changes rounds (500 changes each) in one delta pass; the rest
/// follows in another pass.
const DELTA_ROUNDS_PER_PASS: usize = 40;

fn newest_first() -> Value {
    json!([{ "property": "receivedAt", "isAscending": false }])
}

fn utc_date(d: DateTime<Utc>) -> String {
    d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn id_list(v: &Value) -> Vec<String> {
    v.as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(String::from)).collect()
}

impl JmapProvider {
    /// The recent window from scratch, newest first: new messages are
    /// downloaded, ones we have are brought up to date, and ones in the
    /// window the server no longer lists are removed. Older mail follows in
    /// [`Self::older_page`] (0039).
    pub(super) async fn full_sync(&self, ctx: &SyncCtx, acc: &str) -> Result<()> {
        let cutoff = Utc::now() - chrono::Duration::days(ctx.settings.initial_sync_days.max(7) as i64);
        let filter = json!({ "after": utc_date(cutoff) });
        let mut ids: Vec<String> = vec![];
        let mut position = 0usize;
        let mut capped = false;
        for page in 0.. {
            if page == LIST_PAGES_PER_PASS {
                capped = true;
                break;
            }
            let r = self
                .call(vec![json!(["Email/query", { "accountId": acc, "filter": filter, "sort": newest_first(), "position": position, "limit": 1000, "calculateTotal": true }, "q"])])
                .await?;
            let q = &r["q"];
            let page = id_list(&q["ids"]);
            let n = page.len();
            ids.extend(page);
            position += n;
            ctx.progress(format!("Listing messages ({})", ids.len()));
            if n == 0 || q["total"].as_u64().is_some_and(|t| position as u64 >= t) {
                break;
            }
        }
        // State to resume deltas from.
        let st = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": [], "properties": ["id"] }, "s"])]).await?;
        let email_state = st["s"]["state"].as_str().unwrap_or_default().to_string();
        // A window too big to list in one pass (or a server that never stops
        // paging) is cut at the last message listed; backfill takes it from
        // there, and only the listed span is checked for removals.
        let mut window_start = cutoff;
        if capped && let Some(last) = ids.last() {
            let r = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": [last], "properties": ["receivedAt"] }, "l"])]).await?;
            let at = r["l"]["list"][0]["receivedAt"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc));
            window_start = at.map_or(Utc::now(), |d| d + chrono::Duration::seconds(1)).max(cutoff);
            warn!("JMAP: stopped listing after {} messages; older ones follow as backfill", ids.len());
        }
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let remote: std::collections::HashSet<&String> = ids.iter().collect();
        for rid in ctx.store.remote_ids_since(ctx.account_id(), window_start)? {
            if !remote.contains(&rid) {
                ctx.delete_remote(&rid)?;
            }
        }
        let missing: Vec<String> = ids.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        let existing: Vec<String> = ids.iter().filter(|i| known.contains_key(*i)).cloned().collect();
        let total = missing.len();
        for (i, chunk) in missing.chunks(500).enumerate() {
            ctx.progress(format!("Downloading headers {}/{}", i * 500, total));
            self.get_emails(ctx, acc, chunk).await?;
        }
        // Refresh flags/mailboxes of messages we already had.
        for chunk in existing.chunks(500) {
            let r = self.call(vec![json!(["Email/get", { "accountId": acc, "ids": chunk, "properties": ["id", "mailboxIds", "keywords"] }, "f"])]).await?;
            for e in r["f"]["list"].as_array().cloned().unwrap_or_default() {
                let rid = e["id"].as_str().unwrap_or_default();
                ctx.set_flags_remote(rid, flags_of(&e))?;
                ctx.set_memberships_remote(rid, &mailboxes_of(&e))?;
            }
        }
        ctx.set_cursor("jmap:email_state", &email_state)?;
        ctx.set_cursor("jmap:backfill", &utc_date(window_start))?;
        if capped {
            ctx.more_to_sync();
        }
        Ok(())
    }

    /// One page of older mail, newest first. The cursor is the date the
    /// next page ends before, or "done". Returns whether more is left.
    pub(super) async fn older_page(&self, ctx: &SyncCtx, acc: &str) -> Result<bool> {
        const PAGE: usize = 500;
        let Some(before) = ctx.cursor("jmap:backfill")?.filter(|c| c != "done") else { return Ok(false) };
        let r = self
            .call(vec![
                json!(["Email/query", { "accountId": acc, "filter": { "before": before }, "sort": newest_first(), "limit": PAGE }, "q"]),
                json!(["Email/get", { "accountId": acc, "#ids": { "resultOf": "q", "name": "Email/query", "path": "/ids" }, "properties": ["id", "receivedAt"] }, "d"]),
            ])
            .await?;
        let ids = id_list(&r["q"]["ids"]);
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let missing: Vec<String> = ids.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        ctx.progress(format!("Older mail before {}", &before[..10.min(before.len())]));
        self.get_emails(ctx, acc, &missing).await?;
        let oldest = r["d"]["list"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| DateTime::parse_from_rfc3339(e["receivedAt"].as_str()?).ok())
            .map(|d| d.with_timezone(&Utc))
            .min();
        let next = match oldest {
            Some(o) if ids.len() >= PAGE => {
                // Overlap by a second so mail sharing the oldest timestamp
                // isn't skipped; unless a whole page shares it.
                let n = utc_date(o + chrono::Duration::seconds(1));
                if n == before { utc_date(o) } else { n }
            }
            _ => "done".to_string(),
        };
        ctx.set_cursor("jmap:backfill", &next)?;
        Ok(next != "done")
    }

    pub(super) async fn delta_sync(&self, ctx: &SyncCtx, acc: &str, since: &str) -> Result<()> {
        let mut since = since.to_string();
        for round in 1.. {
            let r = self.call(vec![json!(["Email/changes", { "accountId": acc, "sinceState": since, "maxChanges": 500 }, "c"])]).await?;
            let c = &r["c"];
            let ids = |k: &str| -> Vec<String> { c[k].as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(String::from)).collect() };
            let mut changed = ids("created");
            changed.extend(ids("updated"));
            for d in ids("destroyed") {
                ctx.delete_remote(&d)?;
            }
            if !changed.is_empty() {
                self.get_emails(ctx, acc, &changed).await?;
            }
            let new_state = c["newState"].as_str().unwrap_or(&since).to_string();
            // A state that doesn't move would loop forever.
            let stuck = new_state == since;
            since = new_state;
            ctx.set_cursor("jmap:email_state", &since)?;
            if c["hasMoreChanges"] != true || stuck {
                break;
            }
            if round == DELTA_ROUNDS_PER_PASS {
                ctx.more_to_sync();
                break;
            }
        }
        Ok(())
    }
}
