//! Thread files and the per-account and per-mailbox indexes.

use super::*;

impl Mirror<'_> {
    /// Write (or remove) one thread. Returns true if written.
    pub fn write_thread(&self, thread_id: &str) -> Result<bool> {
        let Some(t) = self.store.get_thread_summary(thread_id)? else {
            self.remove_thread(thread_id)?;
            return Ok(false);
        };
        let cutoff = self.cutoff();
        if !self.account_allowed(&t.account_id) || cutoff.is_some_and(|c| t.last_date < c) {
            self.remove_thread(thread_id)?;
            return Ok(false);
        }
        let Some(acc) = self.store.get_account(&t.account_id)? else { return Ok(false) };
        // Each message is held to the date limit too, as `get_thread` does: a
        // new reply mustn't bring a years-old conversation back with it, not
        // even its subject.
        let msgs = self.visible_messages(thread_id)?;
        let t = if cutoff.is_some() { crate::mail::visible::summarize(t, &msgs) } else { Some(t) };
        let (Some(t), Some(first)) = (t, msgs.first()) else {
            self.remove_thread(thread_id)?;
            return Ok(false);
        };
        let rel = PathBuf::from("mail")
            .join(safe_name(&acc.email))
            .join("threads")
            .join(first.summary.date.format("%Y-%m").to_string())
            .join(format!("{}-{}", slug(&satchel_mime::normalize_subject(&t.subject)), &thread_id[thread_id.len().saturating_sub(8)..]));
        if let Some(old) = self.store.mirror_path(thread_id)?
            && old != rel.to_string_lossy() {
                let _ = remove_dir_all(&self.root(), &self.root().join(old));
            }
        let dir = self.root().join(&rel);
        let mailboxes: Vec<Mailbox> = self.store.list_mailboxes(Some(&t.account_id))?;
        let mb_name = |id: &String| mailboxes.iter().find(|m| &m.id == id).map(|m| one_line(&m.name)).unwrap_or_default();
        let mut thread_mbs: Vec<&String> = vec![];
        for id in msgs.iter().flat_map(|m| m.summary.mailbox_ids.iter()) {
            if !thread_mbs.contains(&id) {
                thread_mbs.push(id);
            }
        }

        let mut md = String::new();
        md.push_str("---\n");
        md.push_str(&format!("thread_id: {}\n", thread_id));
        md.push_str(&format!("account_id: {}\n", acc.id));
        md.push_str(&format!("account: {}\n", acc.email));
        md.push_str(&format!("subject: {:?}\n", t.subject));
        md.push_str(&format!(
            "mailboxes: [{}]\n",
            thread_mbs.iter().map(|id| mb_name(id)).collect::<Vec<_>>().join(", ")
        ));
        let unread = msgs.iter().filter(|m| !m.summary.flags.seen).count();
        md.push_str(&format!("messages: {}\nunread: {}\nlast_date: {}\n", msgs.len(), unread, t.last_date.to_rfc3339()));
        md.push_str("---\n\n");
        md.push_str(&format!("# {}\n\n", if t.subject.trim().is_empty() { "(no subject)".to_string() } else { one_line(&t.subject) }));
        let mut meta_msgs = vec![];
        // The files written this pass; others in the folder (from messages
        // now outside the date limit) are removed after.
        let mut written: HashSet<PathBuf> = HashSet::new();
        for (n, m) in msgs.iter().enumerate() {
            let raw = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten());
            let parsed = raw.as_deref().and_then(satchel_mime::parse);
            let body = parsed.as_ref().map(|p| satchel_mime::strip_quotes(&p.text)).unwrap_or_else(|| m.summary.snippet.clone());
            let from = m.summary.from.as_ref().map(|a| one_line(&a.display())).unwrap_or_else(|| "(unknown)".into());
            md.push_str(&format!("## {} — {}\n\n", m.summary.date.format("%Y-%m-%d %H:%M UTC"), from));
            md.push_str(&format!("- message_id: `{}`\n", m.summary.id));
            if !m.summary.to.is_empty() {
                md.push_str(&format!("- to: {}\n", one_line(&fmt_addrs(&m.summary.to))));
            }
            if !m.cc.is_empty() {
                md.push_str(&format!("- cc: {}\n", one_line(&fmt_addrs(&m.cc))));
            }
            let flags = &m.summary.flags;
            let mut fl = vec![];
            if !flags.seen {
                fl.push("unread");
            }
            if flags.flagged {
                fl.push("flagged");
            }
            if flags.answered {
                fl.push("answered");
            }
            if flags.draft {
                fl.push("draft");
            }
            if !fl.is_empty() {
                md.push_str(&format!("- flags: {}\n", fl.join(", ")));
            }
            let atts: Vec<&Attachment> = m.attachments.iter().filter(|a| !a.inline).collect();
            if !atts.is_empty() {
                md.push_str(&format!(
                    "- attachments: {}\n",
                    atts.iter().map(|a| format!("{} ({}, {})", one_line(&a.filename), one_line(&a.mime_type), human_size(a.size))).collect::<Vec<_>>().join("; ")
                ));
            }
            if parsed.as_ref().is_some_and(|p| !p.calendar_parts.is_empty()) {
                md.push_str("- contains a calendar invitation\n");
            }
            // The body is the sender's text: fenced, so it can't pass itself
            // off as another message or as Satchel's own notes.
            if parsed.is_none() {
                md.push_str("- body: not downloaded yet; this is only a preview\n");
            }
            md.push('\n');
            md.push_str(&fenced(body.trim()));
            md.push('\n');
            if self.settings.include_eml
                && RAW_FILES_EXPORTED
                && let Some(r) = &raw {
                    let p = dir.join(format!("{}.eml", n + 1));
                    write_if_changed(&self.root(), &p, r)?;
                    written.insert(p);
                }
            if self.settings.include_attachments
                && RAW_FILES_EXPORTED
                && let Some(r) = &raw {
                    let mut names = Names::reserving(&[]);
                    for a in &atts {
                        if a.size > 25 * 1024 * 1024 {
                            continue;
                        }
                        if let Some((name, _, data)) = satchel_mime::attachment_bytes(r, &a.id) {
                            let safe = safe_name(&name);
                            let (stem, ext) = match safe.rsplit_once('.') {
                                Some((s, e)) if !s.is_empty() && e.len() <= 10 => (s.to_string(), format!(".{e}")),
                                _ => (safe.clone(), String::new()),
                            };
                            let file = names.take(&format!("{}-{stem}", n + 1), &ext, &format!("{}\0{name}", a.id));
                            let p = dir.join("attachments").join(file);
                            write_if_changed(&self.root(), &p, &data)?;
                            written.insert(p);
                        }
                    }
                }
            meta_msgs.push(serde_json::json!({
                "message_id": m.summary.id,
                "date": m.summary.date,
                "from": m.summary.from,
                "to": m.summary.to,
                "cc": m.cc,
                "subject": m.summary.subject,
                "flags": m.summary.flags,
                "mailboxes": m.summary.mailbox_ids.iter().map(mb_name).collect::<Vec<_>>(),
                "attachments": atts,
                "header_message_id": m.message_id_hdr,
            }));
        }
        // Senders of the messages shown, not of the whole thread (as the store
        // does it: the first message's recipients if there are none).
        let mut participants: Vec<&Address> = vec![];
        for a in msgs.iter().filter_map(|m| m.summary.from.as_ref()) {
            if !participants.iter().any(|p| p.email.eq_ignore_ascii_case(&a.email)) {
                participants.push(a);
            }
        }
        if participants.is_empty() {
            participants = first.summary.to.iter().collect();
        }
        let meta = serde_json::json!({
            "thread_id": thread_id,
            "account_id": acc.id,
            "account": acc.email,
            "subject": t.subject,
            "participants": participants,
            "mailboxes": thread_mbs.iter().map(|id| mb_name(id)).collect::<Vec<_>>(),
            "unread": unread,
            "flagged": msgs.iter().any(|m| m.summary.flags.flagged),
            "last_date": t.last_date,
            "messages": meta_msgs,
        });
        write_if_changed(&self.root(), &dir.join("thread.md"), md.as_bytes())?;
        write_if_changed(&self.root(), &dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?.as_slice())?;
        for sub in [dir.clone(), dir.join("attachments")] {
            let files = match list_files(&self.root(), &sub) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                r => r?,
            };
            for f in files {
                let ours = sub != dir || f.extension().is_some_and(|e| e == "eml");
                if ours && !written.contains(&f) {
                    remove_file(&self.root(), &f)?;
                }
            }
        }
        self.store.set_mirror_path(thread_id, Some(&rel.to_string_lossy()))?;
        Ok(true)
    }

    pub fn remove_thread(&self, thread_id: &str) -> Result<()> {
        if let Some(p) = self.store.mirror_path(thread_id)? {
            let _ = remove_dir_all(&self.root(), &self.root().join(&p));
            self.store.set_mirror_path(thread_id, None)?;
        }
        Ok(())
    }

    pub fn cleanup_orphans(&self) -> Result<()> {
        for (t, p) in self.store.orphan_mirror_paths()? {
            let _ = remove_dir_all(&self.root(), &self.root().join(&p));
            self.store.set_mirror_path(&t, None)?;
        }
        Ok(())
    }

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
