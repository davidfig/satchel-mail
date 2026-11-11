//! Which messages have their attachments in the AI folder (0078). Those
//! files are the only copy Satchel keeps; it never reads them back.

use super::*;

impl Store {
    /// Messages dated `since` or later, newest first, in a thread the AI
    /// folder has written, that have attachment parts but none written yet:
    /// (message id, thread id, account id).
    pub fn ai_attachments_wanted(&self, since_ms: Option<i64>, limit: u32) -> Result<Vec<(String, String, String)>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT m.id, m.thread_id, m.account_id FROM messages m JOIN mirror_paths p ON p.thread_id=m.thread_id
                 WHERE m.ai_attachments=0 AND m.attachments_json<>'[]' AND m.date>=? ORDER BY m.date DESC LIMIT ?",
            )?;
            let rows = s.query_map(params![since_ms.unwrap_or(i64::MIN), limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// The thread's messages whose attachments are written.
    pub fn ai_attachments_written(&self, thread_id: &str) -> Result<HashSet<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id FROM messages WHERE thread_id=? AND ai_attachments=1")?;
            let rows = s.query_map([thread_id], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
            Ok(rows)
        })
    }

    pub fn set_ai_attachments(&self, message_ids: &[String], written: bool) -> Result<()> {
        self.w(|c| {
            let mut s = c.prepare("UPDATE messages SET ai_attachments=? WHERE id=?")?;
            for id in message_ids {
                s.execute(params![written, id])?;
            }
            Ok(())
        })
    }

    /// Forget the attachments written for messages dated before
    /// `before_ms` (all of them with None): the attachment window has moved
    /// past them. Returns their threads, to be written again without them.
    pub fn clear_ai_attachments_before(&self, before_ms: Option<i64>) -> Result<Vec<String>> {
        let before = before_ms.unwrap_or(i64::MAX);
        self.w(|c| {
            let mut s = c.prepare("SELECT DISTINCT thread_id FROM messages WHERE ai_attachments=1 AND date<?")?;
            let threads = s.query_map([before], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            c.execute("UPDATE messages SET ai_attachments=0 WHERE ai_attachments=1 AND date<?", [before])?;
            Ok(threads)
        })
    }

    /// How many messages' attachments are still to be written (for
    /// `sync.json`).
    pub fn ai_attachments_pending(&self, since_ms: Option<i64>) -> Result<u64> {
        self.r(|c| {
            Ok(c.query_row::<i64, _, _>(
                "SELECT count(*) FROM messages m JOIN mirror_paths p ON p.thread_id=m.thread_id
                 WHERE m.ai_attachments=0 AND m.attachments_json<>'[]' AND m.date>=?",
                [since_ms.unwrap_or(i64::MIN)],
                |r| r.get(0),
            )? as u64)
        })
    }
}
