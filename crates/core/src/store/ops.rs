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

    pub fn op_reset(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE ops SET status='pending', attempts=0, next_attempt=0 WHERE id=?", [id])?;
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
