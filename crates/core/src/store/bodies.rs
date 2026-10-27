//! Stored message copies: which messages have one, how much disk they take,
//! and freeing the oldest when an account is over its storage limit.

use super::*;

/// A stored copy of a message, as written by [`Store::set_blob`].
pub struct StoredBlob<'a> {
    pub hash: &'a str,
    /// Bytes on disk.
    pub size: u64,
    /// Leaves out attachment contents.
    pub partial: bool,
    pub snippet: &'a str,
    /// From the full message, so sizes are right even when `partial`.
    pub attachments: &'a [Attachment],
}

impl Store {
    pub fn set_blob(&self, message_id: &str, b: &StoredBlob) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE messages SET blob_hash=?, partial=?, blob_size=?, snippet=CASE WHEN ?='' THEN snippet ELSE ? END,
                   attachments_json=?, has_attachments=?, indexed=0, mirrored=0 WHERE id=?",
                params![b.hash, b.partial, b.size as i64, b.snippet, b.snippet, to_json(&b.attachments), b.attachments.iter().any(|a| !a.inline), message_id],
            )?;
            Ok(())
        })?;
        if let Some(t) = self.thread_of(message_id)? {
            self.recompute_thread(&t)?;
        }
        Ok(())
    }

    /// Messages with no stored copy, newest first, as (id, date, server size).
    pub fn messages_without_body(&self, account_id: &str, since: Option<DateTime<Utc>>, limit: u32) -> Result<Vec<(String, DateTime<Utc>, u64)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id, date, size FROM messages WHERE account_id=? AND blob_hash IS NULL AND date>=? ORDER BY date DESC LIMIT ?")?;
            let rows = s
                .query_map(params![account_id, since.map_or(i64::MIN, |d| d.timestamp_millis()), limit], |r| {
                    Ok((r.get(0)?, Utc.timestamp_millis_opt(r.get(1)?).single().unwrap_or_default(), r.get::<_, i64>(2)?.max(0) as u64))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Bytes an account's stored copies take on disk.
    pub fn stored_bytes(&self, account_id: &str) -> Result<u64> {
        self.r(|c| {
            let n: i64 = c.query_row(
                "SELECT COALESCE(SUM(CASE WHEN blob_size>0 THEN blob_size ELSE size END), 0) FROM messages WHERE account_id=? AND blob_hash IS NOT NULL",
                [account_id],
                |r| r.get(0),
            )?;
            Ok(n.max(0) as u64)
        })
    }

    /// The account's oldest message with a stored copy, dated before
    /// `before` if given, as (id, blob hash, bytes).
    pub fn oldest_with_body(&self, account_id: &str, before: Option<DateTime<Utc>>) -> Result<Option<(String, String, u64)>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT id, blob_hash, CASE WHEN blob_size>0 THEN blob_size ELSE size END FROM messages
                 WHERE account_id=? AND blob_hash IS NOT NULL AND date<? ORDER BY date LIMIT 1",
                params![account_id, before.map_or(i64::MAX, |d| d.timestamp_millis())],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)?.max(0) as u64)),
            )
            .optional()?)
        })
    }

    /// Forget a message's stored copy. Its search index entry and AI folder
    /// file stay as they are; opening it fetches it again.
    pub fn drop_blob(&self, message_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE messages SET blob_hash=NULL, partial=0, blob_size=0 WHERE id=?", [message_id])?;
            Ok(())
        })
    }

    /// Whether any message still uses this stored copy.
    pub fn blob_in_use(&self, hash: &str) -> Result<bool> {
        self.r(|c| Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE blob_hash=?)", [hash], |r| r.get(0))?))
    }
}
