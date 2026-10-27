//! Reading messages, and the indexing and mirroring bookkeeping.

use super::*;

impl Store {
    /// Remote ids of all messages in an account (for full-resync diffs).
    pub fn remote_ids(&self, account_id: &str) -> Result<HashMap<String, String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id, id FROM messages WHERE account_id=?")?;
            let rows = s.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    /// Remote ids of an account's messages dated `since` or later.
    pub fn remote_ids_since(&self, account_id: &str, since: DateTime<Utc>) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id FROM messages WHERE account_id=? AND date>=?")?;
            let rows = s.query_map(params![account_id, since.timestamp_millis()], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn thread_of(&self, message_id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT thread_id FROM messages WHERE id=?", [message_id], |r| r.get(0)).optional()?))
    }

    pub fn message_ref(&self, message_id: &str) -> Result<Option<MessageRef>> {
        let base = self.r(|c| {
            Ok(c.query_row(
                "SELECT id, account_id, remote_id, remote_thread_id, message_id_hdr, seen, flagged, answered, draft, blob_hash, version, size FROM messages WHERE id=?",
                [message_id],
                |r| {
                    Ok(MessageRef {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        remote_id: r.get(2)?,
                        remote_thread_id: r.get(3)?,
                        message_id_hdr: r.get(4)?,
                        mailboxes: vec![],
                        flags: Flags { seen: r.get(5)?, flagged: r.get(6)?, answered: r.get(7)?, draft: r.get(8)? },
                        blob_hash: r.get(9)?,
                        version: r.get(10)?,
                        size: r.get::<_, i64>(11)?.max(0) as u64,
                    })
                },
            )
            .optional()?)
        })?;
        let Some(mut m) = base else { return Ok(None) };
        m.mailboxes = self.r(|c| {
            let mut s = c.prepare(
                "SELECT mm.mailbox_id, mb.remote_id, mm.uid FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id WHERE mm.message_id=?",
            )?;
            let rows = s.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        Ok(Some(m))
    }

    fn summary_row(r: &Row) -> rusqlite::Result<MessageSummary> {
        let from_email: Option<String> = r.get("from_email")?;
        Ok(MessageSummary {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            thread_id: r.get("thread_id")?,
            subject: r.get("subject")?,
            from: from_email.map(|e| Address { name: r.get("from_name").ok().flatten(), email: e }),
            to: from_json(&r.get::<_, String>("to_json")?),
            date: ms_to_dt(r.get("date")?),
            snippet: r.get("snippet")?,
            flags: Flags { seen: r.get("seen")?, flagged: r.get("flagged")?, answered: r.get("answered")?, draft: r.get("draft")? },
            mailbox_ids: vec![],
            has_attachments: r.get("has_attachments")?,
        })
    }

    fn fill_mailbox_ids(c: &Connection, msgs: &mut [MessageSummary]) -> Result<()> {
        let mut s = c.prepare_cached("SELECT mailbox_id FROM message_mailboxes WHERE message_id=?")?;
        for m in msgs.iter_mut() {
            m.mailbox_ids = s.query_map([&m.id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        }
        Ok(())
    }

    /// Everything stored about a message except bodies.
    pub fn message_row(&self, id: &str) -> Result<Option<StoredMessage>> {
        self.r(|c| {
            let row = c
                .query_row("SELECT * FROM messages WHERE id=?", [id], |r| {
                    Ok(StoredMessage {
                        summary: Self::summary_row(r)?,
                        cc: from_json(&r.get::<_, String>("cc_json")?),
                        bcc: from_json(&r.get::<_, String>("bcc_json")?),
                        reply_to: from_json(&r.get::<_, String>("reply_to_json")?),
                        references: from_json(&r.get::<_, String>("refs_json")?),
                        message_id_hdr: r.get("message_id_hdr")?,
                        in_reply_to: r.get("in_reply_to")?,
                        attachments: from_json(&r.get::<_, String>("attachments_json")?),
                        blob_hash: r.get("blob_hash")?,
                        partial: r.get("partial")?,
                        remote_id: r.get("remote_id")?,
                        version: r.get("version")?,
                    })
                })
                .optional()?;
            let Some(mut row) = row else { return Ok(None) };
            Self::fill_mailbox_ids(c, std::slice::from_mut(&mut row.summary))?;
            Ok(Some(row))
        })
    }

    pub fn thread_message_ids(&self, thread_id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id FROM messages WHERE thread_id=? ORDER BY date, id")?;
            let rows = s.query_map([thread_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn message_version(&self, id: &str) -> Result<Option<i64>> {
        self.r(|c| Ok(c.query_row("SELECT version FROM messages WHERE id=?", [id], |r| r.get(0)).optional()?))
    }

    /// Messages whose body still needs fetching, newest first.
    /// Messages whose search entry is missing or out of date (headers are
    /// indexed right away; bodies once downloaded).
    pub fn unindexed(&self, limit: u32) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id FROM messages WHERE indexed=0 LIMIT ?")?;
            let rows = s.query_map([limit], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    /// Write search entries. A message is marked indexed only if its body
    /// didn't change while we were reading it.
    pub fn index_messages(&self, docs: &[SearchDoc]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            {
                let mut rowid = tx.prepare_cached("SELECT rowid FROM messages WHERE id=?")?;
                let mut del = tx.prepare_cached("DELETE FROM message_fts WHERE rowid=?")?;
                let mut ins = tx.prepare_cached("INSERT INTO message_fts (rowid, subject, participants, body, attachments) VALUES (?,?,?,?,?)")?;
                let mut mark = tx.prepare_cached("UPDATE messages SET indexed=1 WHERE id=? AND blob_hash IS ?")?;
                for d in docs {
                    let Some(r): Option<i64> = rowid.query_row([&d.id], |r| r.get(0)).optional()? else { continue };
                    del.execute([r])?;
                    ins.execute(params![r, d.subject, d.participants, d.body, d.attachments])?;
                    mark.execute(params![d.id, d.blob_hash])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn unmirrored_threads(&self, limit: u32) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT DISTINCT thread_id FROM messages WHERE mirrored=0 LIMIT ?")?;
            let rows = s.query_map([limit], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn mark_thread_mirrored(&self, thread_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET mirrored=1 WHERE thread_id=?", [thread_id])?;
            Ok(())
        })
    }

    pub fn reset_mirrored(&self) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET mirrored=0", [])?;
            Ok(())
        })
    }

    /// Drop and rebuild the search index (also required after any VACUUM,
    /// which can renumber message rowids).
    pub fn reset_indexed(&self) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM message_fts", [])?;
            c.execute("UPDATE messages SET indexed=0", [])?;
            Ok(())
        })
    }
}
