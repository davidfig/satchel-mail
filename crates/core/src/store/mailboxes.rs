//! Mailboxes (folders and labels).

use super::*;

impl Store {
    /// Point a local mailbox at a new remote id (after a create or an IMAP rename).
    pub fn set_mailbox_remote_id(&self, id: &str, remote_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE mailboxes SET remote_id=? WHERE id=?", params![remote_id, id])?;
            Ok(())
        })
    }

    /// After a server folder rename, point the account's message remote ids
    /// of the form `uidvalidity:uid:mailbox` (IMAP) at the new name, so ops
    /// on those messages select the folder that exists now.
    pub fn rekey_mailbox_messages(&self, account_id: &str, old: &str, new: &str) -> Result<usize> {
        self.w(|c| {
            let tx = c.transaction()?;
            let rows: Vec<(String, String)> = {
                let mut st = tx.prepare("SELECT id, remote_id FROM messages WHERE account_id=? AND length(remote_id)>? AND substr(remote_id, -?)=?")?;
                let suffix = format!(":{old}");
                let n = suffix.chars().count() as i64;
                st.query_map(params![account_id, n, n, suffix], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
            };
            let mut changed = 0;
            for (id, rid) in rows {
                let mut it = rid.splitn(3, ':');
                let (Some(uv), Some(uid), Some(mb)) = (it.next(), it.next(), it.next()) else { continue };
                if mb != old || uv.parse::<u32>().is_err() || uid.parse::<u32>().is_err() {
                    continue;
                }
                tx.execute("UPDATE messages SET remote_id=? WHERE id=?", params![format!("{uv}:{uid}:{new}"), id])?;
                changed += 1;
            }
            tx.commit()?;
            Ok(changed)
        })
    }

    /// Rename a mailbox and move it (and everything under it) to `new_path`.
    pub fn rename_mailbox(&self, id: &str, name: &str, new_path: &str) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            let (account, old): (String, String) = tx.query_row("SELECT account_id, path FROM mailboxes WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            tx.execute("UPDATE mailboxes SET name=?, path=? WHERE id=?", params![name, new_path, id])?;
            // Children: "Old/Child" becomes "New/Child".
            tx.execute(
                "UPDATE mailboxes SET path = ?1 || substr(path, length(?2) + 1) WHERE account_id=?3 AND substr(path, 1, length(?2) + 1) = ?2 || '/'",
                params![new_path, old, account],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn upsert_mailbox(&self, account_id: &str, remote_id: &str, name: &str, path: &str, role: MailboxRole, is_label: bool) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM mailboxes WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE mailboxes SET name=?, path=?, role=?, is_label=? WHERE id=?",
                    params![name, path, role_str(role), is_label, id],
                )?;
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO mailboxes (id, account_id, remote_id, name, path, role, is_label) VALUES (?,?,?,?,?,?,?)",
                params![id, account_id, remote_id, name, path, role_str(role), is_label],
            )?;
            Ok(id)
        })
    }

    fn mailbox_row(r: &Row) -> rusqlite::Result<Mailbox> {
        Ok(Mailbox {
            id: r.get("id")?,
            account_id: r.get("account_id")?,
            name: r.get("name")?,
            path: r.get("path")?,
            role: parse_role(&r.get::<_, String>("role")?),
            is_label: r.get("is_label")?,
            unread: r.get("unread")?,
            total: r.get("total")?,
        })
    }

    pub fn list_mailboxes(&self, account_id: Option<&str>) -> Result<Vec<Mailbox>> {
        self.r(|c| {
            let order = "ORDER BY account_id, CASE role WHEN 'inbox' THEN 0 WHEN 'flagged' THEN 1 WHEN 'drafts' THEN 2 WHEN 'sent' THEN 3 WHEN 'archive' THEN 4 WHEN 'all' THEN 5 WHEN 'junk' THEN 6 WHEN 'trash' THEN 7 ELSE 8 END, path";
            let rows = match account_id {
                Some(a) => {
                    let mut s = c.prepare(&format!("SELECT * FROM mailboxes WHERE account_id=? {order}"))?;
                    s.query_map([a], Self::mailbox_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
                None => {
                    let mut s = c.prepare(&format!("SELECT * FROM mailboxes {order}"))?;
                    s.query_map([], Self::mailbox_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
            };
            Ok(rows)
        })
    }

    pub fn get_mailbox(&self, id: &str) -> Result<Option<Mailbox>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM mailboxes WHERE id=?", [id], Self::mailbox_row).optional()?))
    }

    pub fn mailbox_remote_id(&self, id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT remote_id FROM mailboxes WHERE id=?", [id], |r| r.get(0)).optional()?))
    }

    /// Every message in a mailbox.
    pub fn mailbox_message_ids(&self, id: &str) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT message_id FROM message_mailboxes WHERE mailbox_id=?")?;
            let rows = s.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn mailbox_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<Mailbox>> {
        self.r(|c| {
            Ok(c.query_row("SELECT * FROM mailboxes WHERE account_id=? AND remote_id=?", [account_id, remote_id], Self::mailbox_row)
                .optional()?)
        })
    }

    pub fn mailbox_by_role(&self, account_id: &str, role: MailboxRole) -> Result<Option<Mailbox>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT * FROM mailboxes WHERE account_id=? AND role=? ORDER BY length(path) LIMIT 1",
                [account_id, role_str(role)],
                Self::mailbox_row,
            )
            .optional()?)
        })
    }

    /// Remove mailboxes not in `keep_remote_ids`.
    /// Drop local mailboxes the server no longer lists. Folders created here
    /// and not yet on the server (`PENDING_MAILBOX` ids) stay.
    pub fn prune_mailboxes(&self, account_id: &str, keep_remote_ids: &[String]) -> Result<()> {
        let keep: HashSet<&String> = keep_remote_ids.iter().collect();
        let all = self.list_mailboxes(Some(account_id))?;
        let ids: Vec<String> = all
            .into_iter()
            .filter_map(|m| {
                let rid = self.mailbox_remote_id(&m.id).ok().flatten()?;
                (!keep.contains(&rid) && !rid.starts_with(PENDING_MAILBOX)).then_some(m.id)
            })
            .collect();
        self.remove_mailboxes(account_id, &ids)
    }

    /// Delete local mailboxes, and messages left in no mailbox at all.
    pub fn remove_mailboxes(&self, account_id: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let affected = self.w(|c| {
            let tx = c.transaction()?;
            let mut threads = HashSet::new();
            for id in ids {
                let mut s = tx.prepare("SELECT m.thread_id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id WHERE mm.mailbox_id=?")?;
                for t in s.query_map([id], |r| r.get::<_, String>(0))? {
                    threads.insert(t?);
                }
                drop(s);
                tx.execute("DELETE FROM mailboxes WHERE id=?", [id])?;
            }
            // Messages now without any mailbox are gone from the server.
            let freed = Self::blob_hashes_tx(
                &tx,
                "account_id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes mm WHERE mm.message_id=messages.id)",
                [account_id],
            )?;
            self.blobs_freed(freed);
            tx.execute(
                "DELETE FROM messages WHERE account_id=? AND NOT EXISTS (SELECT 1 FROM message_mailboxes mm WHERE mm.message_id=messages.id)",
                [account_id],
            )?;
            tx.commit()?;
            Ok(threads)
        })?;
        for t in affected {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// Unread messages in a mailbox, or in every mailbox with this role.
    pub fn unread_ids(&self, mailbox_id: Option<&str>, role: Option<MailboxRole>) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT DISTINCT m.id FROM messages m JOIN message_mailboxes mm ON mm.message_id=m.id JOIN mailboxes mb ON mb.id=mm.mailbox_id
                 WHERE m.seen=0 AND (mb.id=?1 OR mb.role=?2)",
            )?;
            let rows = s.query_map(params![mailbox_id, role.map(role_str)], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    pub fn recount_mailboxes(&self, account_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE mailboxes SET
                   total = (SELECT COUNT(*) FROM message_mailboxes mm WHERE mm.mailbox_id = mailboxes.id),
                   unread = (SELECT COUNT(*) FROM message_mailboxes mm JOIN messages m ON m.id = mm.message_id WHERE mm.mailbox_id = mailboxes.id AND m.seen = 0)
                 WHERE account_id = ?",
                [account_id],
            )?;
            Ok(())
        })
    }
}
