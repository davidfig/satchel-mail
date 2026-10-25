//! Full and delta sync of Email objects.

use super::*;

impl JmapProvider {
    pub(super) async fn full_sync(&self, ctx: &SyncCtx, acc: &str) -> Result<()> {
        // Page through every email id, newest first.
        let mut ids: Vec<String> = vec![];
        let mut position = 0usize;
        let mut state = String::new();
        loop {
            let r = self
                .call(vec![json!(["Email/query", { "accountId": acc, "sort": [{ "property": "receivedAt", "isAscending": false }], "position": position, "limit": 1000, "calculateTotal": true }, "q"])])
                .await?;
            let q = &r["q"];
            if state.is_empty() {
                state = q["queryState"].as_str().unwrap_or_default().to_string();
            }
            let page: Vec<String> = q["ids"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
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
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let remote: std::collections::HashSet<&String> = ids.iter().collect();
        for (rid, local) in &known {
            if !remote.contains(rid) {
                ctx.store.delete_message(local)?;
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
        Ok(())
    }

    pub(super) async fn delta_sync(&self, ctx: &SyncCtx, acc: &str, since: &str) -> Result<()> {
        let mut since = since.to_string();
        loop {
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
            since = c["newState"].as_str().unwrap_or(&since).to_string();
            ctx.set_cursor("jmap:email_state", &since)?;
            if c["hasMoreChanges"] != true {
                break;
            }
        }
        Ok(())
    }
}
