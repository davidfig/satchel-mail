//! Threads: listing, filtering and recomputing.

use super::*;

impl Store {
    pub fn recompute_thread(&self, thread_id: &str) -> Result<()> {
        self.w(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, subject, date, seen, flagged, has_attachments, snippet, from_name, from_email, to_json, message_id_hdr, draft
                 FROM messages WHERE thread_id=? ORDER BY date",
            )?;
            struct M {
                id: String,
                account: String,
                subject: String,
                date: i64,
                seen: bool,
                flagged: bool,
                att: bool,
                snippet: String,
                from: Option<Address>,
                to: Vec<Address>,
                mid: Option<String>,
            }
            let rows: Vec<M> = s
                .query_map([thread_id], |r| {
                    let fe: Option<String> = r.get(9)?;
                    Ok(M {
                        id: r.get(0)?,
                        account: r.get(1)?,
                        subject: r.get(2)?,
                        date: r.get(3)?,
                        seen: r.get(4)?,
                        flagged: r.get(5)?,
                        att: r.get(6)?,
                        snippet: r.get(7)?,
                        from: fe.map(|e| Address { name: r.get(8).ok().flatten(), email: e }),
                        to: from_json(&r.get::<_, String>(10)?),
                        mid: r.get(11)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(s);
            if rows.is_empty() {
                c.execute("DELETE FROM threads WHERE id=?", [thread_id])?;
                return Ok(());
            }
            // Count copies of one message (the same Message-ID in several folders) once.
            let mut seen: HashMap<&str, Vec<&M>> = HashMap::new();
            let mut unique: Vec<&M> = vec![];
            for m in &rows {
                if let Some(mid) = m.mid.as_deref() {
                    let kept = seen.entry(mid).or_default();
                    let from = m.from.as_ref().map(|a| a.email.as_str());
                    if kept.iter().any(|k| super::message_reads::is_copy(k.from.as_ref().map(|a| a.email.as_str()), &k.snippet, from, &m.snippet)) {
                        continue;
                    }
                    kept.push(m);
                }
                unique.push(m);
            }
            let last = rows.last().unwrap();
            let first = &rows[0];
            let mut participants: Vec<Address> = vec![];
            for m in &unique {
                if let Some(f) = &m.from
                    && !participants.iter().any(|p| p.email.eq_ignore_ascii_case(&f.email)) {
                        participants.push(f.clone());
                    }
            }
            if participants.is_empty() {
                participants = first.to.clone();
            }
            let subject = if first.subject.is_empty() { last.subject.clone() } else { first.subject.clone() };
            c.execute(
                "INSERT INTO threads (id, account_id, subject, last_date, message_count, unread_count, flagged, has_attachments, snippet, latest_message_id, participants_json)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET subject=excluded.subject, last_date=excluded.last_date, message_count=excluded.message_count,
                   unread_count=excluded.unread_count, flagged=excluded.flagged, has_attachments=excluded.has_attachments,
                   snippet=excluded.snippet, latest_message_id=excluded.latest_message_id, participants_json=excluded.participants_json",
                params![
                    thread_id,
                    last.account,
                    subject,
                    last.date,
                    unique.len() as i64,
                    unique.iter().filter(|m| !m.seen).count() as i64,
                    rows.iter().any(|m| m.flagged),
                    rows.iter().any(|m| m.att),
                    last.snippet,
                    last.id,
                    to_json(&participants)
                ],
            )?;
            Ok(())
        })
    }

    fn thread_row(r: &Row) -> rusqlite::Result<ThreadSummary> {
        Ok(ThreadSummary {
            thread_id: r.get("id")?,
            account_id: r.get("account_id")?,
            subject: r.get("subject")?,
            participants: from_json(&r.get::<_, String>("participants_json")?),
            last_date: ms_to_dt(r.get("last_date")?),
            message_count: r.get("message_count")?,
            unread_count: r.get("unread_count")?,
            flagged: r.get("flagged")?,
            has_attachments: r.get("has_attachments")?,
            snippet: r.get("snippet")?,
            mailbox_ids: vec![],
            latest_message_id: r.get("latest_message_id")?,
            favorite: false,
        })
    }

    /// What the threads table doesn't hold: the thread's mailboxes, and
    /// whether a favorite sender wrote any of it.
    fn fill_thread_extras(c: &Connection, t: &mut ThreadSummary) -> Result<()> {
        let mut ms = c.prepare_cached(
            "SELECT DISTINCT mm.mailbox_id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id WHERE m.thread_id=?",
        )?;
        t.mailbox_ids = ms.query_map([&t.thread_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        let mut fav = c.prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM messages m JOIN favorite_senders fs ON fs.email=lower(m.from_email) WHERE m.thread_id=?)",
        )?;
        t.favorite = fav.query_row([&t.thread_id], |r| r.get(0))?;
        Ok(())
    }

    /// A message condition matching any of `values` as sender and/or
    /// recipient (To, Cc, Bcc). Full addresses match exactly; anything else
    /// matches part of a name or address.
    fn any_address(values: &[String], sender: bool, recipient: bool, p: &mut Vec<rusqlite::types::Value>) -> String {
        use rusqlite::types::Value as V;
        let mut ors = vec![];
        let like = |s: &str| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        for v in values {
            let v = v.to_lowercase();
            let exact = crate::mail::search::is_full_address(&v);
            if sender {
                if exact {
                    ors.push("lower(m.from_email)=?");
                    p.push(V::Text(v.clone()));
                } else {
                    ors.push(r"lower(m.from_email) LIKE ? ESCAPE '\' OR lower(m.from_name) LIKE ? ESCAPE '\'");
                    p.push(V::Text(like(&v)));
                    p.push(V::Text(like(&v)));
                }
            }
            if recipient {
                // Addresses are stored as JSON objects; an exact match looks for the whole email value.
                let needle = if exact { like(&format!("\"email\":{}", serde_json::Value::String(v.clone()))) } else { like(&v) };
                ors.push(r"lower(m.to_json) LIKE ? ESCAPE '\' OR lower(m.cc_json) LIKE ? ESCAPE '\' OR lower(m.bcc_json) LIKE ? ESCAPE '\'");
                for _ in 0..3 {
                    p.push(V::Text(needle.clone()));
                }
            }
        }
        format!("({})", ors.join(" OR "))
    }

    /// WHERE clause + params for a thread filter (excluding paging). `None`
    /// means the filter can match nothing.
    fn thread_where(c: &Connection, f: &ThreadFilter) -> Result<Option<(Vec<String>, Vec<rusqlite::types::Value>)>> {
        let mut where_: Vec<String> = vec![];
        let mut p: Vec<rusqlite::types::Value> = vec![];
        use rusqlite::types::Value as V;
        let mut msg_conds: Vec<String> = vec![];
        // Small mailboxes: drive from the membership index (avoids scanning
        // every thread). Large ones: walk threads newest-first and stop early.
        let total_threads: i64 = c.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))?;
        let size: i64 = match (&f.mailbox_id, f.role) {
            (Some(mb), _) => c.query_row("SELECT total FROM mailboxes WHERE id=?", [mb], |r| r.get(0)).unwrap_or(0),
            (None, Some(role)) => c.query_row("SELECT COALESCE(SUM(total),0) FROM mailboxes WHERE role=?", [role_str(role)], |r| r.get(0)).unwrap_or(0),
            _ => 0,
        };
        let sparse = size * 10 < total_threads;
        // Clauses (and their params) that precede the message-level EXISTS.
        let mut pre_where: Vec<String> = vec![];
        let mut pre_p: Vec<rusqlite::types::Value> = vec![];
        if let Some(mb) = &f.mailbox_id {
            if sparse {
                pre_where.push("t.id IN (SELECT m2.thread_id FROM message_mailboxes mm2 JOIN messages m2 ON m2.id=mm2.message_id WHERE mm2.mailbox_id=?)".into());
                pre_p.push(V::Text(mb.clone()));
            } else {
                msg_conds.push("EXISTS (SELECT 1 FROM message_mailboxes mm WHERE mm.message_id=m.id AND mm.mailbox_id=?)".into());
                p.push(V::Text(mb.clone()));
            }
        }
        if let Some(role) = f.role {
            if sparse {
                pre_where.push("t.id IN (SELECT m2.thread_id FROM message_mailboxes mm2 JOIN messages m2 ON m2.id=mm2.message_id WHERE mm2.mailbox_id IN (SELECT id FROM mailboxes WHERE role=?))".into());
                pre_p.push(V::Text(role_str(role).into()));
            } else {
                msg_conds.push("EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND mb.role=?)".into());
                p.push(V::Text(role_str(role).into()));
            }
        }
        if let Some(names) = &f.in_mailbox_names {
            let ph = vec!["?"; names.len()].join(",");
            msg_conds.push(format!(
                "EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND (lower(mb.name) IN ({ph}) OR mb.role IN ({ph})))"
            ));
            for n in names {
                p.push(V::Text(n.to_lowercase()));
            }
            for n in names {
                p.push(V::Text(n.to_lowercase()));
            }
        }
        if f.exclude_trash_junk {
            msg_conds.push("NOT EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=m.id AND mb.role IN ('trash','junk'))".into());
        }
        if let Some(a) = &f.account_ids {
            // On the thread, not the message: inside the EXISTS, SQLite picks
            // the account index over messages_thread and scans the account's
            // whole mailbox once per thread (seconds on a real account).
            let ph = vec!["?"; a.len()].join(",");
            pre_where.push(format!("t.account_id IN ({ph})"));
            for x in a {
                pre_p.push(V::Text(x.clone()));
            }
        }
        if !f.from.is_empty() {
            msg_conds.push(Self::any_address(&f.from, true, false, &mut p));
        }
        if !f.to.is_empty() {
            msg_conds.push(Self::any_address(&f.to, false, true, &mut p));
        }
        if !f.with.is_empty() {
            msg_conds.push(Self::any_address(&f.with, true, true, &mut p));
        }
        if let Some(s) = &f.subject {
            msg_conds.push("lower(m.subject) LIKE ?".into());
            p.push(V::Text(format!("%{}%", s.to_lowercase())));
        }
        if f.has_attachment {
            msg_conds.push("m.has_attachments=1".into());
        }
        if f.unread {
            msg_conds.push("m.seen=0".into());
        }
        if f.flagged {
            msg_conds.push("m.flagged=1".into());
        }
        if f.favorite {
            msg_conds.push("EXISTS (SELECT 1 FROM favorite_senders fs WHERE fs.email=lower(m.from_email))".into());
        }
        if let Some(d) = f.after {
            msg_conds.push("m.date>=?".into());
            p.push(V::Integer(d.timestamp_millis()));
        }
        if let Some(d) = f.msg_before {
            msg_conds.push("m.date<?".into());
            p.push(V::Integer(d.timestamp_millis()));
        }
        if let Some(q) = &f.fts {
            // Selective text (few matches): start from the text index. Broad text:
            // walk threads newest-first and stop early. Either way the text and the
            // other filters must hold for the same message.
            let probe: i64 = c.query_row(
                "SELECT COUNT(*) FROM (SELECT rowid FROM message_fts WHERE message_fts MATCH ? LIMIT 5001)",
                [q],
                |r| r.get(0),
            )?;
            if probe == 0 {
                return Ok(None);
            }
            if probe <= 5000 {
                pre_where.push("t.id IN (SELECT m3.thread_id FROM messages m3 WHERE m3.rowid IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ?))".into());
                pre_p.push(V::Text(q.clone()));
            }
            msg_conds.push("m.rowid IN (SELECT rowid FROM message_fts WHERE message_fts MATCH ?)".into());
            p.push(V::Text(q.clone()));
        }
        if let Some(ids) = &f.message_ids {
            if ids.is_empty() {
                return Ok(None);
            }
            let ph = vec!["?"; ids.len()].join(",");
            msg_conds.push(format!("m.id IN ({ph})"));
            for x in ids {
                p.push(V::Text(x.clone()));
            }
        }
        if !msg_conds.is_empty() {
            where_.push(format!("EXISTS (SELECT 1 FROM messages m WHERE m.thread_id=t.id AND {})", msg_conds.join(" AND ")));
        }
        pre_where.append(&mut where_);
        pre_p.append(&mut p);
        Ok(Some((pre_where, pre_p)))
    }

    /// List threads newest first. Paging is either keyset (`before` +
    /// `before_id`, stable when threads share a timestamp) or by `offset` (random
    /// access for the windowed list UI).
    pub fn list_threads(&self, f: &ThreadFilter) -> Result<Vec<ThreadSummary>> {
        use rusqlite::types::Value as V;
        self.r(|c| {
            let Some((mut where_, mut p)) = Self::thread_where(c, f)? else { return Ok(vec![]) };
            if let Some(b) = f.before {
                match &f.before_id {
                    Some(id) => {
                        where_.push("(t.last_date<? OR (t.last_date=? AND t.id<?))".into());
                        p.push(V::Integer(b.timestamp_millis()));
                        p.push(V::Integer(b.timestamp_millis()));
                        p.push(V::Text(id.clone()));
                    }
                    None => {
                        where_.push("t.last_date<?".into());
                        p.push(V::Integer(b.timestamp_millis()));
                    }
                }
            }
            let sql = format!(
                "SELECT t.* FROM threads t {} ORDER BY t.last_date DESC, t.id DESC LIMIT ? OFFSET ?",
                if where_.is_empty() { String::new() } else { format!("WHERE {}", where_.join(" AND ")) }
            );
            p.push(V::Integer(f.limit as i64));
            p.push(V::Integer(f.offset as i64));
            let mut s = c.prepare(&sql)?;
            let mut rows = s.query_map(params_from_iter(p.iter()), Self::thread_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            for t in rows.iter_mut() {
                Self::fill_thread_extras(c, t)?;
            }
            Ok(rows)
        })
    }

    /// Number of threads a filter matches (ignores paging).
    pub fn count_threads(&self, f: &ThreadFilter) -> Result<u32> {
        self.r(|c| {
            let Some((where_, p)) = Self::thread_where(c, f)? else { return Ok(0) };
            let sql = format!(
                "SELECT COUNT(*) FROM threads t {}",
                if where_.is_empty() { String::new() } else { format!("WHERE {}", where_.join(" AND ")) }
            );
            Ok(c.query_row(&sql, params_from_iter(p.iter()), |r| r.get(0))?)
        })
    }

    pub fn get_thread_summary(&self, id: &str) -> Result<Option<ThreadSummary>> {
        self.r(|c| {
            let Some(mut t) = c.query_row("SELECT * FROM threads WHERE id=?", [id], Self::thread_row).optional()? else { return Ok(None) };
            Self::fill_thread_extras(c, &mut t)?;
            Ok(Some(t))
        })
    }

    pub fn mirror_path(&self, thread_id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT path FROM mirror_paths WHERE thread_id=?", [thread_id], |r| r.get(0)).optional()?))
    }

    pub fn set_mirror_path(&self, thread_id: &str, path: Option<&str>) -> Result<()> {
        self.w(|c| {
            match path {
                Some(p) => c.execute(
                    "INSERT INTO mirror_paths (thread_id, path) VALUES (?,?) ON CONFLICT(thread_id) DO UPDATE SET path=excluded.path",
                    params![thread_id, p],
                )?,
                None => c.execute("DELETE FROM mirror_paths WHERE thread_id=?", [thread_id])?,
            };
            Ok(())
        })
    }

    /// Mirror entries whose thread no longer exists.
    pub fn orphan_mirror_paths(&self) -> Result<Vec<(String, String)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT thread_id, path FROM mirror_paths WHERE thread_id NOT IN (SELECT id FROM threads)")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
