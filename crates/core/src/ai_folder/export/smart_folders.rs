//! `smart-folders.md`: the user's saved searches and the threads in each.

use super::*;

/// Threads listed per smart folder.
const PER_FOLDER: u32 = 100;

impl Mirror<'_> {
    pub fn write_smart_folders(&self) -> Result<()> {
        let path = self.root().join("smart-folders.md");
        let folders = self.store.list_smart_folders()?;
        // A folder's name and query can describe any account's mail, so a
        // folder limited to some accounts leaves them out altogether.
        let every_account = self.store.list_accounts()?.iter().all(|a| self.account_allowed(&a.id));
        if folders.is_empty() || !every_account {
            let _ = remove_file(&self.root(), &path);
            return Ok(());
        }
        let cutoff = self.cutoff();
        let cell = |s: &str| one_line(s).replace('|', "/");
        // A thread is often in several folders; its summary is the same in each.
        let mut visible = HashMap::<String, Option<ThreadSummary>>::new();
        let mut s = String::from(
            "# Smart folders\n\nThe user's saved searches, across every folder of every account. \
             Each query is in the search language of the `search_threads` MCP tool.\n",
        );
        for f in &folders {
            let mut filter = crate::store::ThreadFilter { account_ids: self.settings.account_ids.clone(), after: cutoff, limit: PER_FOLDER, ..Default::default() };
            crate::mail::read::apply_search(self.store, &mut filter, &f.query)?;
            // Under a date limit each row counts, names and previews only the
            // messages inside it, and a thread with none there isn't listed.
            let mut threads = vec![];
            for t in self.store.list_threads(&filter)? {
                let id = t.thread_id.clone();
                let summary = match visible.get(&id) {
                    Some(s) => s.clone(),
                    None => {
                        let s = self.visible_summary(t)?;
                        visible.insert(id, s.clone());
                        s
                    }
                };
                threads.extend(summary);
            }
            s.push_str(&format!("\n## {}\n\nQuery: `{}`\n\n", one_line(&f.name), one_line(&f.query).replace('`', "'")));
            if threads.is_empty() {
                s.push_str("No threads.\n");
                continue;
            }
            s.push_str("| date | from | subject | msgs | unread | thread |\n|---|---|---|---|---|---|\n");
            for t in &threads {
                let id = &t.thread_id[t.thread_id.len().saturating_sub(8)..];
                // Paths are relative to the folder's root, where this file is.
                let link = match self.store.mirror_path(&t.thread_id)?.filter(|p| self.root().join(p).join("thread.md").is_file()) {
                    Some(path) => format!("[{id}]({path}/thread.md)"),
                    None => format!("{id} (not written yet)"),
                };
                let from = t.participants.first().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    t.last_date.format("%Y-%m-%d"),
                    cell(&from),
                    cell(&t.subject),
                    t.message_count,
                    t.unread_count,
                    link
                ));
            }
        }
        write_if_changed(&self.root(), &path, s.as_bytes())?;
        Ok(())
    }
}
