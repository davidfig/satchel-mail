//! The per-account `INDEX.md` and `index.jsonl`, and per-mailbox listings.

use super::*;

impl Mirror<'_> {
    /// Per-account INDEX.md and per-mailbox listings. An account that fails
    /// is logged and skipped, so the others are still written.
    pub fn write_indexes(&self) -> Result<()> {
        let accounts = self.store.list_accounts()?;
        for acc in &accounts {
            if let Err(e) = self.write_account_indexes(acc) {
                tracing::warn!("AI folder: indexes for {}: {e:#}", acc.email);
            }
        }
        // Folders of accounts that were removed (or written by another data
        // directory) would read as live mail.
        let mail = self.root().join("mail");
        let ours: HashSet<String> = accounts.iter().map(|a| fold(&safe_name(&a.email))).collect();
        if let Ok(entries) = std::fs::read_dir(&mail) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().is_ok_and(|t| t.is_dir()) && !ours.contains(&fold(&name)) {
                    let _ = remove_dir_all(&self.root(), &entry.path());
                }
            }
        }
        Ok(())
    }

    fn write_account_indexes(&self, acc: &Account) -> Result<()> {
        let base = self.root().join("mail").join(safe_name(&acc.email));
        if !self.account_allowed(&acc.id) {
            let _ = remove_dir_all(&self.root(), &base);
            return Ok(());
        }
        let mailboxes = self.store.list_mailboxes(Some(&acc.id))?;
        let prefix = format!("mail/{}/", safe_name(&acc.email));
        // Where each thread's folder is, relative to the account folder, if
        // its thread.md is really there. One that was written but has gone
        // (deleted by hand, or a rewrite cut short) is queued to be written
        // again, and listed without a link until it is.
        let missing = std::cell::RefCell::new(vec![]);
        let folder = |thread_id: &str| -> Result<Option<String>> {
            let Some(path) = self.store.mirror_path(thread_id)? else { return Ok(None) };
            if !self.root().join(&path).join("thread.md").is_file() {
                missing.borrow_mut().push(thread_id.to_string());
                return Ok(None);
            }
            Ok(Some(path.strip_prefix(&prefix).unwrap_or(&path).to_string()))
        };
        let from_of = |t: &ThreadSummary| t.participants.first().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
        let render = |title: &str, threads: &[ThreadSummary]| -> Result<String> {
            let cell = |s: &str| one_line(s).replace('|', "/");
            let mut s = format!("# {}\n\n| date | from | subject | msgs | unread | thread |\n|---|---|---|---|---|---|\n", one_line(title));
            for t in threads {
                let id = &t.thread_id[t.thread_id.len().saturating_sub(8)..];
                let link = match folder(&t.thread_id)? {
                    Some(rel) => format!("[{id}]({rel}/thread.md)"),
                    None => format!("{id} (not written yet)"),
                };
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    t.last_date.format("%Y-%m-%d"),
                    cell(&from_of(t)),
                    cell(&t.subject),
                    t.message_count,
                    t.unread_count,
                    link
                ));
            }
            Ok(s)
        };
        // Under a date limit each row counts, names and previews only the
        // messages inside it, and a thread with none there isn't listed.
        let cutoff = self.cutoff();
        let visible = std::cell::RefCell::new(HashMap::<String, Option<ThreadSummary>>::new());
        let filter = |v: Vec<ThreadSummary>, mailbox: Option<&str>| -> Result<Vec<ThreadSummary>> {
            if cutoff.is_none() {
                return Ok(v);
            }
            let mut out = vec![];
            for t in v {
                let cached = visible.borrow().get(&t.thread_id).cloned();
                let s = match cached {
                    Some(s) => s,
                    None => {
                        let id = t.thread_id.clone();
                        let s = self.visible_summary(t)?;
                        visible.borrow_mut().insert(id, s.clone());
                        s
                    }
                };
                if let Some(s) = s.filter(|s| mailbox.is_none_or(|mb| s.mailbox_ids.iter().any(|x| x == mb))) {
                    out.push(s);
                }
            }
            Ok(out)
        };
        let all = filter(
            self.store.list_threads(&crate::store::ThreadFilter {
                account_ids: Some(vec![acc.id.clone()]),
                exclude_trash_junk: true,
                after: cutoff,
                limit: 200,
                ..Default::default()
            })?,
            None,
        )?;
        write_if_changed(&self.root(), &base.join("INDEX.md"), render(&format!("{} — recent threads", acc.email), &all)?.as_bytes())?;
        // The same rows for tools: one JSON object per line.
        let mut jsonl = String::new();
        for t in &all {
            let row = serde_json::json!({
                "thread_id": t.thread_id,
                "last_date": t.last_date,
                "from": from_of(t),
                "subject": t.subject,
                "messages": t.message_count,
                "unread": t.unread_count,
                "flagged": t.flagged,
                "favorite": t.favorite,
                "mailboxes": t.mailbox_ids.iter().filter_map(|id| mailboxes.iter().find(|m| &m.id == id).map(|m| m.path.clone())).collect::<Vec<_>>(),
                "path": folder(&t.thread_id)?.map(|rel| format!("{rel}/thread.md")),
            });
            jsonl.push_str(&serde_json::to_string(&row)?);
            jsonl.push('\n');
        }
        write_if_changed(&self.root(), &base.join("index.jsonl"), jsonl.as_bytes())?;
        // A mailbox called "index" (or two differing only in case) mustn't
        // overwrite INDEX.md or each other.
        let mut names = Names::reserving(&["INDEX.md", "index.jsonl", "threads"]);
        for mb in &mailboxes {
            let file = base.join(names.take(&safe_name(&mb.path), ".md", &mb.id));
            let threads = filter(
                self.store.list_threads(&crate::store::ThreadFilter {
                    mailbox_id: Some(mb.id.clone()),
                    after: cutoff,
                    limit: 200,
                    ..Default::default()
                })?,
                Some(&mb.id),
            )?;
            // An empty mailbox gets no file: it would only clutter listings.
            let r = if threads.is_empty() {
                match remove_file(&self.root(), &file) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                    _ => Ok(()),
                }
            } else {
                write_if_changed(&self.root(), &file, render(&mb.path, &threads)?.as_bytes())
            };
            if let Err(e) = r {
                tracing::warn!("AI folder: index of {}: {e:#}", mb.path);
            }
        }
        let mut missing = missing.into_inner();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            tracing::debug!("AI folder: {} thread folders missing in {}, writing them again", missing.len(), acc.email);
            self.store.mark_threads_unmirrored(&missing)?;
        }
        Ok(())
    }
}
