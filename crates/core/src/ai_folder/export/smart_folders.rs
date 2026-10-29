//! `smart-folders.md`: the user's saved searches and the threads in each.

use super::*;

/// Threads listed per smart folder.
const PER_FOLDER: u32 = 100;

impl Mirror<'_> {
    pub fn write_smart_folders(&self) -> Result<()> {
        let path = self.root().join("smart-folders.md");
        let folders = self.store.list_smart_folders()?;
        if folders.is_empty() {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        let cutoff = self.settings.since_days.map(|d| Utc::now() - Duration::days(d as i64));
        let mut s = String::from(
            "# Smart folders\n\nThe user's saved searches, across every folder of every account. \
             Each query is in the search language of the `search_threads` MCP tool.\n",
        );
        for f in &folders {
            let mut filter = crate::store::ThreadFilter { account_ids: self.settings.account_ids.clone(), after: cutoff, limit: PER_FOLDER, ..Default::default() };
            crate::mail::read::apply_search(self.store, &mut filter, &f.query)?;
            let threads = self.store.list_threads(&filter)?;
            s.push_str(&format!("\n## {}\n\nQuery: `{}`\n\n", f.name.replace('\n', " "), f.query.replace('`', "'")));
            if threads.is_empty() {
                s.push_str("No threads.\n");
                continue;
            }
            s.push_str("| date | from | subject | msgs | unread | thread |\n|---|---|---|---|---|---|\n");
            for t in &threads {
                let link = self.store.mirror_path(&t.thread_id)?.unwrap_or_default();
                let from = t.participants.first().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | [{}]({}/thread.md) |\n",
                    t.last_date.format("%Y-%m-%d"),
                    from.replace('|', "/"),
                    t.subject.replace('|', "/"),
                    t.message_count,
                    t.unread_count,
                    &t.thread_id[t.thread_id.len().saturating_sub(8)..],
                    link
                ));
            }
        }
        write_if_changed(&self.root(), &path, s.as_bytes())?;
        Ok(())
    }
}
