//! Thread files, and keeping them in step with the store and the date limit.

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
        let attachment_window = self.attachment_window();
        // Messages whose attachments were downloaded for the folder: their
        // files are kept, not written again. Afterwards, `on` and `off` say
        // which messages now have all their files, and which lost them.
        let downloaded = self.store.ai_attachments_written(thread_id)?;
        let (mut on, mut off) = (vec![], vec![]);
        // The files written this pass; others in the folder (from messages
        // now outside the date limit) are removed after.
        let mut written: HashSet<PathBuf> = HashSet::new();
        for (n, m) in msgs.iter().enumerate() {
            let raw = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten());
            let parsed = raw.as_deref().and_then(satchel_mime::parse);
            let body = parsed.as_ref().map(satchel_mime::agent_body).unwrap_or_else(|| m.summary.snippet.clone());
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
            let atts: Vec<&Attachment> = m.attachments.iter().filter(|a| listed_attachment(a)).collect();
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
            let in_window = attachment_window.is_some_and(|since| since.is_none_or(|s| m.summary.date >= s));
            let mut complete = in_window;
            if in_window {
                // The stored copy has the contents unless it's slimmed.
                let full = raw.as_deref().filter(|_| !m.partial);
                for (id, file) in attachment_files(n + 1, &atts) {
                    let p = dir.join("attachments").join(file);
                    if let Some((_, _, data)) = full.and_then(|r| satchel_mime::attachment_bytes(r, &id)) {
                        write_if_changed(&self.root(), &p, &data)?;
                    } else if !(downloaded.contains(&m.summary.id) && p.is_file()) {
                        complete = false;
                        continue;
                    }
                    written.insert(p);
                }
            }
            match (complete, downloaded.contains(&m.summary.id)) {
                (true, false) => on.push(m.summary.id.clone()),
                // Out of the window, or a file went missing: the background
                // download fetches it again if it's still wanted.
                (false, true) => off.push(m.summary.id.clone()),
                _ => {}
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
        self.store.set_ai_attachments(&on, true)?;
        self.store.set_ai_attachments(&off, false)?;
        Ok(true)
    }

    pub fn remove_thread(&self, thread_id: &str) -> Result<()> {
        if let Some(p) = self.store.mirror_path(thread_id)? {
            let _ = remove_dir_all(&self.root(), &self.root().join(&p));
            self.store.set_mirror_path(thread_id, None)?;
        }
        Ok(())
    }

    /// Queue every thread whose folder was written but has gone (deleted by
    /// hand, by another program, or by a rewrite cut short) to be written
    /// again. The indexes only notice this for the threads they list.
    /// Returns how many were missing.
    pub fn check_threads(&self) -> Result<usize> {
        let root = self.root();
        let missing: Vec<String> =
            self.store.mirror_paths()?.into_iter().filter(|(_, p)| !root.join(p).join("thread.md").is_file()).map(|(t, _)| t).collect();
        if !missing.is_empty() {
            tracing::info!("AI folder: {} thread folders missing, writing them again", missing.len());
            self.store.mark_threads_unmirrored(&missing)?;
        }
        Ok(missing.len())
    }

    /// Queue every written thread the date limit has moved past, in part or
    /// whole, to be written again: that drops its older messages, or removes
    /// its folder when none are left. Threads are otherwise only rewritten
    /// when their mail changes, so without this a conversation would stay in
    /// the folder long after it aged out. Returns how many were queued.
    pub fn sweep_aged(&self) -> Result<usize> {
        // Attachments past their own window (or all of them, when the folder
        // no longer takes any) go the same way (0078).
        let since = match self.attachment_window() {
            None => None,
            Some(None) => Some(i64::MIN),
            Some(Some(d)) => Some(d.timestamp_millis()),
        };
        let mut aged = self.store.clear_ai_attachments_before(since)?;
        if let Some(cutoff) = self.cutoff() {
            aged.extend(self.store.mirrored_threads_before(cutoff.timestamp_millis())?);
        }
        aged.sort();
        aged.dedup();
        if !aged.is_empty() {
            tracing::debug!("AI folder: {} threads reach past the date limit, writing them again", aged.len());
            self.store.mark_threads_unmirrored(&aged)?;
        }
        Ok(aged.len())
    }

    pub fn cleanup_orphans(&self) -> Result<()> {
        for (t, p) in self.store.orphan_mirror_paths()? {
            let _ = remove_dir_all(&self.root(), &self.root().join(&p));
            self.store.set_mirror_path(&t, None)?;
        }
        Ok(())
    }
}
