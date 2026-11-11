//! `mail/<account>/history/<yyyy-mm>.jsonl`: one line per message, for every
//! month, so agents can count, filter and find across all mail with one
//! pass of `jq` (0076). A month's file is rewritten only when its messages
//! change.

use super::*;

use chrono::{DateTime, Datelike, TimeZone};

/// Bumped when a row's shape changes, so every month is written again.
const FORMAT: u64 = 2;

impl Mirror<'_> {
    /// Write the history index of every account the folder shares. Returns
    /// how many month files were written.
    pub fn write_history(&self) -> Result<usize> {
        let mut n = 0;
        for acc in self.store.list_accounts()? {
            if !self.account_allowed(&acc.id) {
                continue;
            }
            match self.write_account_history(&acc) {
                Ok(w) => n += w,
                Err(e) => tracing::warn!("AI folder: history for {}: {e:#}", acc.email),
            }
        }
        Ok(n)
    }

    /// How far back history goes: all of it, or the text window.
    fn history_since(&self) -> Option<DateTime<Utc>> {
        if self.settings.history_all { None } else { self.cutoff() }
    }

    fn write_account_history(&self, acc: &Account) -> Result<usize> {
        let dir = self.root().join("mail").join(safe_name(&acc.email)).join("history");
        let mailboxes: HashMap<String, String> = self.store.list_mailboxes(Some(&acc.id))?.into_iter().map(|m| (m.id, m.path)).collect();
        let mine: HashSet<String> =
            std::iter::once(acc.email.to_lowercase()).chain(acc.send_as.iter().map(|s| s.email.to_lowercase())).collect();
        // What every row of the account depends on besides its message.
        let mut base = fnv(FORMAT, acc.email.as_bytes());
        let mut names: Vec<_> = mailboxes.iter().collect();
        names.sort();
        for (id, path) in names {
            base = fnv(fnv(base, id.as_bytes()), path.as_bytes());
        }
        let mut mine_sorted: Vec<_> = mine.iter().collect();
        mine_sorted.sort();
        for m in mine_sorted {
            base = fnv(base, m.as_bytes());
        }

        // Each month's fingerprint: the same whatever order rows come in.
        let mut months: HashMap<(i32, u32), u64> = HashMap::new();
        for (date, version, thread) in self.store.history_keys(&acc.id, self.history_since())? {
            let d = crate::store::ms_to_dt(date);
            let h = fnv(fnv(fnv(0, &date.to_le_bytes()), &version.to_le_bytes()), thread.as_bytes());
            let e = months.entry((d.year(), d.month())).or_insert(base);
            *e = e.wrapping_add(h);
        }

        let mut written = 0;
        let mut keep: HashSet<PathBuf> = HashSet::new();
        for (&(y, m), &fp) in &months {
            let file = dir.join(format!("{y:04}-{m:02}.jsonl"));
            keep.insert(file.clone());
            let key = format!("{}/{y:04}-{m:02}", acc.id);
            if self.history_seen.lock().unwrap().get(&key) == Some(&fp) && file.is_file() {
                continue;
            }
            let from = Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single().unwrap_or_default();
            let to = if m == 12 { Utc.with_ymd_and_hms(y + 1, 1, 1, 0, 0, 0) } else { Utc.with_ymd_and_hms(y, m + 1, 1, 0, 0, 0) }.single().unwrap_or_default();
            let since = self.history_since();
            let mut out = String::new();
            for r in self.store.history_rows(&acc.id, from, to)? {
                if since.is_some_and(|s| r.summary.date < s) {
                    continue;
                }
                out.push_str(&serde_json::to_string(&row(&r, &mailboxes, &mine))?);
                out.push('\n');
            }
            write_if_changed(&self.root(), &file, out.as_bytes())?;
            self.history_seen.lock().unwrap().insert(key, fp);
            written += 1;
        }
        // Months with no messages left, or now outside the window.
        match list_files(&self.root(), &dir) {
            Ok(files) => {
                for f in files {
                    if !keep.contains(&f) {
                        remove_file(&self.root(), &f)?;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(written)
    }
}

/// One message's line.
fn row(r: &crate::store::HistoryRow, mailboxes: &HashMap<String, String>, mine: &HashSet<String>) -> serde_json::Value {
    let s = &r.summary;
    let emails = |v: &[Address]| v.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
    serde_json::json!({
        "date": s.date,
        "message_id": s.id,
        "thread_id": s.thread_id,
        "from": s.from.as_ref().map(|a| serde_json::json!({ "name": a.name, "email": a.email })),
        "from_me": s.from.as_ref().is_some_and(|a| mine.contains(&a.email.to_lowercase())),
        "to": emails(&s.to),
        "cc": emails(&r.cc),
        "subject": s.subject,
        "mailboxes": s.mailbox_ids.iter().filter_map(|id| mailboxes.get(id)).collect::<Vec<_>>(),
        "unread": !s.flags.seen,
        "flagged": s.flags.flagged,
        "answered": s.flags.answered,
        "draft": s.flags.draft,
        "attachments": r.attachments.iter().filter(|a| listed_attachment(a)).map(|a| serde_json::json!({ "name": a.filename, "type": a.mime_type, "size": a.size })).collect::<Vec<_>>(),
        "snippet": satchel_mime::agent_text(&s.snippet),
    })
}

/// FNV-1a, continuing from `h`.
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    let mut h = if h == 0 { 0xcbf2_9ce4_8422_2325 } else { h };
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
