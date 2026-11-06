//! The outbound op queue.

use super::*;

impl Store {
    pub fn enqueue_op(&self, account_id: &str, kind: &str, payload: &serde_json::Value, actor: &str) -> Result<String> {
        let id = new_id();
        self.w(|c| {
            c.execute(
                "INSERT INTO ops (id, account_id, kind, payload_json, status, actor, created_at) VALUES (?,?,?,?, 'pending', ?, ?)",
                params![id, account_id, kind, payload.to_string(), actor, now_ms()],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Queue an op and make the local change it stands for in one commit. A
    /// sync writing at the same time then finds either neither, and is
    /// overwritten by the change, or both, and leaves the message alone
    /// ([`Store::change_pending`]). And the op can't run ahead of the change.
    pub fn enqueue_change(&self, account_id: &str, kind: &str, payload: &serde_json::Value, actor: &str, change: LocalChange) -> Result<String> {
        let id = new_id();
        let threads = self.w(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO ops (id, account_id, kind, payload_json, status, actor, created_at) VALUES (?,?,?,?, 'pending', ?, ?)",
                params![id, account_id, kind, payload.to_string(), actor, now_ms()],
            )?;
            let threads = self.apply_change_tx(&tx, &change)?;
            tx.commit()?;
            Ok(threads)
        })?;
        for t in threads {
            self.recompute_thread(&t)?;
        }
        Ok(id)
    }

    /// The ops to run now, in the order they were queued (insertion order
    /// breaks ties within a millisecond). An op waiting to retry holds back
    /// everything queued after it, so a folder is never deleted before the
    /// move that empties it; sends are independent and never wait or block.
    pub fn due_ops(&self, account_id: &str) -> Result<Vec<OpRow>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, kind, payload_json, attempts, actor, next_attempt<=? FROM ops
                 WHERE account_id=? AND status='pending' ORDER BY created_at, rowid",
            )?;
            let rows = s
                .query_map(params![now_ms(), account_id], |r| {
                    Ok((
                        OpRow {
                            id: r.get(0)?,
                            account_id: r.get(1)?,
                            kind: r.get(2)?,
                            payload: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                            attempts: r.get(4)?,
                            actor: r.get(5)?,
                        },
                        r.get::<_, bool>(6)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut out = vec![];
            for (op, due) in rows {
                match (due, op.kind == "send") {
                    (true, _) => out.push(op),
                    (false, true) => continue,
                    (false, false) => break,
                }
            }
            Ok(out)
        })
    }

    /// Whether a queued change to this message (by local or remote id) hasn't
    /// reached the server yet. Sync leaves such a message as the user left it:
    /// the server still shows it the old way until the op runs, and copying
    /// that back would undo the change on screen until the next sync.
    pub fn change_pending(&self, account_id: &str, id: Option<&str>, remote_id: Option<&str>) -> Result<bool> {
        self.r(|c| Ok(Self::change_pending_tx(c, account_id, id, remote_id)?))
    }

    pub(super) fn change_pending_tx(c: &rusqlite::Connection, account_id: &str, id: Option<&str>, remote_id: Option<&str>) -> rusqlite::Result<bool> {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM ops o WHERE o.account_id=?1 AND o.status IN ('pending','sending') AND (
               EXISTS(SELECT 1 FROM json_each(o.payload_json, '$.refs') j
                      WHERE json_extract(j.value, '$.id')=?2 OR json_extract(j.value, '$.remote_id')=?3)
               OR EXISTS(SELECT 1 FROM json_each(o.payload_json, '$.message_ids') j WHERE j.value=?2)))",
            params![account_id, id, remote_id],
            |r| r.get(0),
        )
    }

    pub fn has_pending_ops(&self, account_id: &str) -> Result<bool> {
        self.r(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM ops WHERE account_id=? AND status='pending'", [account_id], |r| r.get(0))?;
            Ok(n > 0)
        })
    }

    pub fn op_done(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='done' WHERE id=?", [id])?;
            // Keep a short history only.
            c.execute("DELETE FROM ops WHERE status='done' AND created_at<?", [now_ms() - 7 * 86_400_000])?;
            Ok(())
        })
    }

    /// Mark a send as in flight. A send still in flight when the account
    /// loop next runs ops was interrupted (crash, quit, account restart) and
    /// may have gone out: see [`Store::fail_interrupted_sends`].
    pub fn op_sending(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='sending' WHERE id=?", [id])?;
            Ok(())
        })
    }

    /// Fail the account's sends that were interrupted mid-flight, returning
    /// them. They are never retried automatically.
    pub fn fail_interrupted_sends(&self, account_id: &str, error: &str) -> Result<Vec<OpRow>> {
        self.w(|c| {
            let mut s = c.prepare("SELECT id, account_id, kind, payload_json, attempts, actor FROM ops WHERE account_id=? AND status='sending'")?;
            let rows = s
                .query_map([account_id], |r| {
                    Ok(OpRow {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        kind: r.get(2)?,
                        payload: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                        attempts: r.get(4)?,
                        actor: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            c.execute("UPDATE ops SET status='failed', last_error=? WHERE account_id=? AND status='sending'", params![error, account_id])?;
            Ok(rows)
        })
    }

    pub fn op_retry(&self, id: &str, error: &str, attempts: u32) -> Result<()> {
        let backoff = (2i64.pow(attempts.min(10)) * 1000).min(15 * 60 * 1000);
        self.w(|c| {
            c.execute(
                "UPDATE ops SET status='pending', attempts=?, last_error=?, next_attempt=? WHERE id=?",
                params![attempts, error, now_ms() + backoff, id],
            )?;
            Ok(())
        })
    }

    pub fn op_failed(&self, id: &str, error: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='failed', last_error=? WHERE id=?", params![error, id])?;
            Ok(())
        })
    }

    /// Retry a failed or waiting op now. One being sent, or done, is left
    /// alone: resetting it could send a message twice.
    pub fn op_reset(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='pending', attempts=0, next_attempt=0 WHERE id=? AND status IN ('failed','pending')", [id])?;
            Ok(())
        })
    }

    pub fn list_outbox(&self) -> Result<Vec<OutboxItem>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT id, account_id, kind, status, attempts, last_error, created_at FROM ops WHERE status<>'done' ORDER BY created_at DESC LIMIT 200",
            )?;
            let rows = s
                .query_map([], |r| {
                    Ok(OutboxItem {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        kind: r.get(2)?,
                        status: r.get(3)?,
                        attempts: r.get(4)?,
                        last_error: r.get(5)?,
                        created_at: ms_to_dt(r.get(6)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_run_in_order_and_a_retrying_op_holds_back_later_ones() {
        let s = Store::open_in_memory().unwrap();
        let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
        let kinds = |s: &Store| s.due_ops(&acc.id).unwrap().into_iter().map(|o| o.kind).collect::<Vec<_>>();
        // Queued within the same millisecond: still in queue order.
        let mv = s.enqueue_op(&acc.id, "move", &serde_json::json!({}), "user").unwrap();
        let send = s.enqueue_op(&acc.id, "send", &serde_json::json!({}), "user").unwrap();
        s.enqueue_op(&acc.id, "mailbox_delete", &serde_json::json!({}), "user").unwrap();
        assert_eq!(kinds(&s), vec!["move", "send", "mailbox_delete"]);
        // A send waiting to retry doesn't hold anything back...
        s.op_retry(&send, "down", 1).unwrap();
        assert_eq!(kinds(&s), vec!["move", "mailbox_delete"]);
        // ...but any other op does, along with everything after it.
        s.op_retry(&mv, "down", 1).unwrap();
        assert!(kinds(&s).is_empty());
    }

    #[test]
    fn only_failed_or_waiting_ops_can_be_retried() {
        let s = Store::open_in_memory().unwrap();
        let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
        let id = s.enqueue_op(&acc.id, "send", &serde_json::json!({}), "user").unwrap();
        s.op_sending(&id).unwrap();
        s.op_reset(&id).unwrap();
        assert!(s.due_ops(&acc.id).unwrap().is_empty(), "a send in flight isn't put back in the queue");
    }
}
