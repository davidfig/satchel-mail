//! Thread files and the per-account and per-mailbox indexes.

use super::*;

impl Mirror<'_> {
    /// Write (or remove) one thread. Returns true if written.
    pub fn write_thread(&self, thread_id: &str) -> Result<bool> {
        let Some(t) = self.store.get_thread_summary(thread_id)? else {
            self.remove_thread(thread_id)?;
            return Ok(false);
        };
        let cutoff = self.settings.since_days.map(|d| Utc::now() - Duration::days(d as i64));
        if !self.account_allowed(&t.account_id) || cutoff.is_some_and(|c| t.last_date < c) {
            self.remove_thread(thread_id)?;
            return Ok(false);
        }
        let Some(acc) = self.store.get_account(&t.account_id)? else { return Ok(false) };
        let ids = self.store.thread_message_ids(thread_id)?;
        let mut msgs = vec![];
        for id in &ids {
            if let Some(m) = self.store.message_row(id)? {
                msgs.push(m);
            }
        }
        let Some(first) = msgs.first() else { return Ok(false) };
        let rel = PathBuf::from("mail")
            .join(safe_name(&acc.email))
            .join("threads")
            .join(first.summary.date.format("%Y-%m").to_string())
            .join(format!("{}-{}", slug(&satchel_mime::normalize_subject(&t.subject)), &thread_id[thread_id.len().saturating_sub(8)..]));
        if let Some(old) = self.store.mirror_path(thread_id)?
            && old != rel.to_string_lossy() {
                let _ = std::fs::remove_dir_all(self.root().join(old));
            }
        let dir = self.root().join(&rel);
        let mailboxes: Vec<Mailbox> = self.store.list_mailboxes(Some(&t.account_id))?;
        let mb_name = |id: &String| mailboxes.iter().find(|m| &m.id == id).map(|m| m.name.clone()).unwrap_or_default();

        let mut md = String::new();
        md.push_str("---\n");
        md.push_str(&format!("thread_id: {}\n", thread_id));
        md.push_str(&format!("account_id: {}\n", acc.id));
        md.push_str(&format!("account: {}\n", acc.email));
        md.push_str(&format!("subject: {:?}\n", t.subject));
        md.push_str(&format!(
            "mailboxes: [{}]\n",
            t.mailbox_ids.iter().map(mb_name).collect::<Vec<_>>().join(", ")
        ));
        md.push_str(&format!("messages: {}\nunread: {}\nlast_date: {}\n", t.message_count, t.unread_count, t.last_date.to_rfc3339()));
        md.push_str("---\n\n");
        md.push_str(&format!("# {}\n\n", if t.subject.is_empty() { "(no subject)" } else { &t.subject }));
        let mut meta_msgs = vec![];
        for (n, m) in msgs.iter().enumerate() {
            let raw = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten());
            let parsed = raw.as_deref().and_then(satchel_mime::parse);
            let body = parsed.as_ref().map(|p| satchel_mime::strip_quotes(&p.text)).unwrap_or_else(|| m.summary.snippet.clone());
            let from = m.summary.from.as_ref().map(|a| a.display()).unwrap_or_else(|| "(unknown)".into());
            md.push_str(&format!("## {} — {}\n\n", m.summary.date.format("%Y-%m-%d %H:%M UTC"), from));
            md.push_str(&format!("- message_id: `{}`\n", m.summary.id));
            if !m.summary.to.is_empty() {
                md.push_str(&format!("- to: {}\n", fmt_addrs(&m.summary.to)));
            }
            if !m.cc.is_empty() {
                md.push_str(&format!("- cc: {}\n", fmt_addrs(&m.cc)));
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
                    atts.iter().map(|a| format!("{} ({}, {})", a.filename, a.mime_type, human_size(a.size))).collect::<Vec<_>>().join("; ")
                ));
            }
            if parsed.as_ref().is_some_and(|p| !p.calendar_parts.is_empty()) {
                md.push_str("- contains a calendar invitation\n");
            }
            md.push('\n');
            md.push_str(body.trim());
            md.push_str("\n\n");
            if self.settings.include_eml
                && let Some(r) = &raw {
                    write_if_changed(&self.root(), &dir.join(format!("{}.eml", n + 1)), r)?;
                }
            if self.settings.include_attachments
                && let Some(r) = &raw {
                    for a in &atts {
                        if a.size > 25 * 1024 * 1024 {
                            continue;
                        }
                        if let Some((name, _, data)) = satchel_mime::attachment_bytes(r, &a.id) {
                            write_if_changed(&self.root(), &dir.join("attachments").join(format!("{}-{}", n + 1, safe_name(&name))), &data)?;
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
        let meta = serde_json::json!({
            "thread_id": thread_id,
            "account_id": acc.id,
            "account": acc.email,
            "subject": t.subject,
            "participants": t.participants,
            "mailboxes": t.mailbox_ids.iter().map(mb_name).collect::<Vec<_>>(),
            "unread": t.unread_count,
            "flagged": t.flagged,
            "last_date": t.last_date,
            "messages": meta_msgs,
        });
        write_if_changed(&self.root(), &dir.join("thread.md"), md.as_bytes())?;
        write_if_changed(&self.root(), &dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?.as_slice())?;
        self.store.set_mirror_path(thread_id, Some(&rel.to_string_lossy()))?;
        Ok(true)
    }

    pub fn remove_thread(&self, thread_id: &str) -> Result<()> {
        if let Some(p) = self.store.mirror_path(thread_id)? {
            let _ = std::fs::remove_dir_all(self.root().join(&p));
            self.store.set_mirror_path(thread_id, None)?;
        }
        Ok(())
    }

    pub fn cleanup_orphans(&self) -> Result<()> {
        for (t, p) in self.store.orphan_mirror_paths()? {
            let _ = std::fs::remove_dir_all(self.root().join(&p));
            self.store.set_mirror_path(&t, None)?;
        }
        Ok(())
    }

    /// Per-account INDEX.md and per-mailbox listings.
    pub fn write_indexes(&self) -> Result<()> {
        for acc in self.store.list_accounts()? {
            if !self.account_allowed(&acc.id) {
                let _ = std::fs::remove_dir_all(self.root().join("mail").join(safe_name(&acc.email)));
                continue;
            }
            let base = self.root().join("mail").join(safe_name(&acc.email));
            let mailboxes = self.store.list_mailboxes(Some(&acc.id))?;
            let render = |title: &str, threads: &[ThreadSummary]| -> Result<String> {
                let mut s = format!("# {title}\n\n| date | from | subject | msgs | unread | thread |\n|---|---|---|---|---|---|\n");
                for t in threads {
                    let path = self.store.mirror_path(&t.thread_id)?.unwrap_or_default();
                    let rel = path.strip_prefix(&format!("mail/{}/", safe_name(&acc.email))).unwrap_or(&path).to_string();
                    let from = t.participants.first().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                    s.push_str(&format!(
                        "| {} | {} | {} | {} | {} | [{}]({}/thread.md) |\n",
                        t.last_date.format("%Y-%m-%d"),
                        from.replace('|', "/"),
                        t.subject.replace('|', "/"),
                        t.message_count,
                        t.unread_count,
                        &t.thread_id[t.thread_id.len().saturating_sub(8)..],
                        rel
                    ));
                }
                Ok(s)
            };
            let cutoff = self.settings.since_days.map(|d| Utc::now() - Duration::days(d as i64));
            let filter = |mut v: Vec<ThreadSummary>| {
                v.retain(|t| cutoff.is_none_or(|c| t.last_date >= c));
                v
            };
            let all = filter(self.store.list_threads(&crate::store::ThreadFilter {
                account_ids: Some(vec![acc.id.clone()]),
                exclude_trash_junk: true,
                limit: 200,
                ..Default::default()
            })?);
            write_if_changed(&self.root(), &base.join("INDEX.md"), render(&format!("{} — recent threads", acc.email), &all)?.as_bytes())?;
            for mb in &mailboxes {
                let threads = filter(self.store.list_threads(&crate::store::ThreadFilter {
                    mailbox_id: Some(mb.id.clone()),
                    limit: 200,
                    ..Default::default()
                })?);
                write_if_changed(&self.root(), &base.join(format!("{}.md", safe_name(&mb.path))), render(&mb.path, &threads)?.as_bytes())?;
            }
        }
        Ok(())
    }
}
