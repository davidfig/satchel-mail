//! Writing messages: upserts, memberships, flags.

use super::*;

/// The most References ids kept for a message (the last ones: the parent and
/// its nearest ancestors). A hostile header can name tens of thousands.
pub const MAX_REFERENCES: usize = 100;

/// The References ids kept for a message.
fn kept_refs(rec: &MessageRecord) -> &[String] {
    &rec.references[rec.references.len().saturating_sub(MAX_REFERENCES)..]
}

impl Store {
    pub fn message_id_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM messages WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?)
        })
    }

    /// Insert or update a message and its mailbox memberships. Returns
    /// (local id, inserted?).
    pub fn upsert_message(&self, account_id: &str, rec: &MessageRecord, mailboxes: &[(String, Option<u32>)]) -> Result<(String, bool)> {
        let (id, inserted, threads) = self.w(|c| {
            let tx = c.transaction()?;
            let existing: Option<(String, String, Option<String>)> = tx
                .query_row("SELECT id, thread_id, blob_hash FROM messages WHERE account_id=? AND remote_id=?", [account_id, &rec.remote_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let mut affected_threads: Vec<String> = vec![];
            let (id, inserted) = if let Some((id, thread_id, old_blob)) = existing {
                if rec.blob_hash.is_some() && old_blob != rec.blob_hash {
                    self.blobs_freed(old_blob);
                }
                tx.execute(
                    "UPDATE messages SET seen=?, flagged=?, answered=?, draft=?, remote_thread_id=COALESCE(?, remote_thread_id),
                       blob_hash=COALESCE(?, blob_hash), snippet=CASE WHEN ?='' THEN snippet ELSE ? END,
                       partial=CASE WHEN ? IS NULL THEN partial ELSE ? END, blob_size=CASE WHEN ? IS NULL THEN blob_size ELSE ? END,
                       attachments_json=CASE WHEN ?='[]' THEN attachments_json ELSE ? END,
                       has_attachments=MAX(has_attachments, ?), version=version+1, mirrored=0
                     WHERE id=?",
                    params![
                        rec.flags.seen, rec.flags.flagged, rec.flags.answered, rec.flags.draft, rec.remote_thread_id,
                        rec.blob_hash, rec.snippet, rec.snippet, rec.blob_hash, rec.partial, rec.blob_hash, rec.blob_size as i64,
                        to_json(&rec.attachments), to_json(&rec.attachments),
                        rec.attachments.iter().any(|a| !a.inline), id
                    ],
                )?;
                affected_threads.push(thread_id);
                (id, false)
            } else {
                let id = new_id();
                let refs = kept_refs(rec);
                let (thread_id, merge) = Self::assign_thread(&tx, account_id, rec)?;
                tx.execute(
                    "INSERT INTO messages (id, account_id, remote_id, remote_thread_id, thread_id, message_id_hdr, in_reply_to, subject,
                       from_name, from_email, to_json, cc_json, bcc_json, reply_to_json, refs_json, date, snippet, seen, flagged, answered,
                       draft, has_attachments, attachments_json, blob_hash, size, partial, blob_size)
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        id, account_id, rec.remote_id, rec.remote_thread_id, thread_id, rec.message_id_hdr, rec.in_reply_to, rec.subject,
                        rec.from.as_ref().and_then(|a| a.name.clone()), rec.from.as_ref().map(|a| a.email.clone()),
                        to_json(&rec.to), to_json(&rec.cc), to_json(&rec.bcc), to_json(&rec.reply_to), to_json(&refs),
                        rec.date.timestamp_millis(), rec.snippet, rec.flags.seen, rec.flags.flagged, rec.flags.answered, rec.flags.draft,
                        rec.attachments.iter().any(|a| !a.inline), to_json(&rec.attachments), rec.blob_hash, rec.size as i64,
                        rec.partial, rec.blob_size as i64
                    ],
                )?;
                let mut refs: Vec<&String> = refs.iter().collect();
                if let Some(irt) = &rec.in_reply_to {
                    refs.push(irt);
                }
                for r in refs.into_iter().collect::<HashSet<_>>() {
                    tx.execute("INSERT INTO message_refs (message_id, account_id, ref) VALUES (?,?,?)", params![id, account_id, r])?;
                }
                for other in &merge {
                    tx.execute("UPDATE messages SET thread_id=? WHERE thread_id=?", params![thread_id, other])?;
                    tx.execute("DELETE FROM threads WHERE id=?", [other])?;
                }
                affected_threads.push(thread_id);
                (id, true)
            };
            Self::set_memberships_tx(&tx, &id, mailboxes)?;
            tx.commit()?;
            Ok((id, inserted, affected_threads))
        })?;
        for t in threads {
            self.recompute_thread(&t)?;
        }
        Ok((id, inserted))
    }

    /// Pick a thread for a new message. Returns (thread id, other threads to merge into it).
    ///
    /// A message naming messages already stored joins the thread of the most
    /// recent one, and never merges threads: anyone can name any Message-ID.
    /// Only threads that all reply to this message (they arrived before it)
    /// are merged, as they belong together whatever this message says.
    fn assign_thread(tx: &rusqlite::Transaction, account_id: &str, rec: &MessageRecord) -> Result<(String, Vec<String>)> {
        if let Some(rt) = &rec.remote_thread_id {
            let t: Option<String> = tx
                .query_row("SELECT thread_id FROM messages WHERE account_id=? AND remote_thread_id=? LIMIT 1", [account_id, rt], |r| r.get(0))
                .optional()?;
            return Ok((t.unwrap_or_else(new_id), vec![]));
        }
        let mut ids: Vec<&String> = kept_refs(rec).iter().collect();
        ids.extend(rec.in_reply_to.iter());
        ids.extend(rec.message_id_hdr.iter());
        if !ids.is_empty() {
            let named: Option<String> = tx
                .query_row(
                    "SELECT thread_id FROM messages WHERE account_id=? AND message_id_hdr IN (SELECT value FROM json_each(?))
                     ORDER BY date DESC LIMIT 1",
                    params![account_id, to_json(&ids)],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(t) = named {
                return Ok((t, vec![]));
            }
        }
        let mut found: Vec<String> = vec![];
        if let Some(mid) = &rec.message_id_hdr {
            // Replies that arrived before their parent.
            let mut s = tx.prepare(
                "SELECT m.thread_id FROM message_refs r JOIN messages m ON m.id=r.message_id WHERE r.account_id=? AND r.ref=?
                 GROUP BY m.thread_id ORDER BY MAX(m.date) DESC",
            )?;
            for t in s.query_map([account_id, mid], |r| r.get::<_, String>(0))? {
                found.push(t?);
            }
        }
        if found.is_empty() {
            return Ok((new_id(), vec![]));
        }
        let first = found.remove(0);
        Ok((first, found))
    }

    fn set_memberships_tx(tx: &rusqlite::Transaction, message_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        tx.execute("DELETE FROM message_mailboxes WHERE message_id=?", [message_id])?;
        for (mb, uid) in mailboxes {
            tx.execute(
                "INSERT OR REPLACE INTO message_mailboxes (message_id, mailbox_id, uid) VALUES (?,?,?)",
                params![message_id, mb, uid],
            )?;
        }
        Ok(())
    }

    /// Replace the remote identity of a message (e.g. IMAP MOVE assigned a new UID).
    pub fn rekey_message(&self, message_id: &str, new_remote_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            // If sync already inserted the new copy, drop it in favour of the existing row.
            let dup: Option<String> = tx
                .query_row(
                    "SELECT id FROM messages WHERE remote_id=? AND account_id=(SELECT account_id FROM messages WHERE id=?) AND id<>?",
                    params![new_remote_id, message_id, message_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(d) = dup {
                self.blobs_freed(Self::blob_hashes_tx(&tx, "id=?", [&d])?);
                tx.execute("DELETE FROM messages WHERE id=?", [d])?;
            }
            tx.execute("UPDATE messages SET remote_id=?, version=version+1, mirrored=0 WHERE id=?", params![new_remote_id, message_id])?;
            Self::set_memberships_tx(&tx, message_id, mailboxes)?;
            tx.commit()?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn set_memberships(&self, message_id: &str, mailboxes: &[(String, Option<u32>)]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            Self::set_memberships_tx(&tx, message_id, mailboxes)?;
            tx.execute("UPDATE messages SET version=version+1, mirrored=0 WHERE id=?", [message_id])?;
            tx.commit()?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn memberships(&self, message_id: &str) -> Result<Vec<(String, Option<u32>)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT mailbox_id, uid FROM message_mailboxes WHERE message_id=?")?;
            let rows = s.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn set_flags(&self, message_id: &str, flags: &Flags) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE messages SET seen=?, flagged=?, answered=?, draft=?, version=version+1, mirrored=0 WHERE id=?",
                params![flags.seen, flags.flagged, flags.answered, flags.draft, message_id],
            )?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    pub fn delete_message(&self, message_id: &str) -> Result<()> {
        let thread = self.thread_of(message_id)?;
        self.w(|c| {
            let tx = c.transaction()?;
            let freed = Self::blob_hashes_tx(&tx, "id=?", [message_id])?;
            tx.execute("DELETE FROM messages WHERE id=?", [message_id])?;
            tx.commit()?;
            self.blobs_freed(freed);
            Ok(())
        })?;
        if let Some(t) = thread {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// Remove a message from one mailbox; deletes it when no memberships remain.
    pub fn remove_from_mailbox(&self, message_id: &str, mailbox_id: &str) -> Result<()> {
        let thread = self.thread_of(message_id)?;
        self.w(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM message_mailboxes WHERE message_id=? AND mailbox_id=?", [message_id, mailbox_id])?;
            let freed = Self::blob_hashes_tx(&tx, "id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes WHERE message_id=?)", [message_id, message_id])?;
            tx.execute(
                "DELETE FROM messages WHERE id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes WHERE message_id=?)",
                [message_id, message_id],
            )?;
            tx.commit()?;
            self.blobs_freed(freed);
            Ok(())
        })?;
        if let Some(t) = thread {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// (uid, message id, flags) for every message in a mailbox.
    pub fn mailbox_uids(&self, mailbox_id: &str) -> Result<Vec<(u32, String, Flags)>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT mm.uid, m.id, m.seen, m.flagged, m.answered, m.draft FROM message_mailboxes mm JOIN messages m ON m.id=mm.message_id WHERE mm.mailbox_id=? AND mm.uid IS NOT NULL",
            )?;
            let rows = s
                .query_map([mailbox_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, Flags { seen: r.get(2)?, flagged: r.get(3)?, answered: r.get(4)?, draft: r.get(5)? }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Messages placed in a mailbox optimistically (no server UID yet).
    pub fn mailbox_unconfirmed(&self, mailbox_id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT message_id FROM message_mailboxes WHERE mailbox_id=? AND uid IS NULL")?;
            let rows = s.query_map([mailbox_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }
}
