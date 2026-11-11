//! Reads for the AI folder's history index: every message's metadata, by
//! month (0076).

use super::*;

/// A message as the history index lists it.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub summary: MessageSummary,
    pub cc: Vec<Address>,
    pub attachments: Vec<Attachment>,
}

impl Store {
    /// (date ms, version, thread id) of each of an account's messages since
    /// `since`: enough to tell when a month's rows change.
    pub fn history_keys(&self, account_id: &str, since: Option<DateTime<Utc>>) -> Result<Vec<(i64, i64, String)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT date, version, thread_id FROM messages WHERE account_id=? AND date>=?")?;
            let rows = s
                .query_map(params![account_id, since.map_or(i64::MIN, |d| d.timestamp_millis())], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// An account's messages dated from `from` up to (not including) `to`,
    /// oldest first.
    pub fn history_rows(&self, account_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<HistoryRow>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM messages WHERE account_id=? AND date>=? AND date<? ORDER BY date, id")?;
            let mut rows = s
                .query_map(params![account_id, from.timestamp_millis(), to.timestamp_millis()], |r| {
                    Ok(HistoryRow {
                        summary: Self::summary_row(r)?,
                        cc: from_json(&r.get::<_, String>("cc_json")?),
                        attachments: from_json(&r.get::<_, String>("attachments_json")?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut s = c.prepare_cached("SELECT mailbox_id FROM message_mailboxes WHERE message_id=?")?;
            for r in rows.iter_mut() {
                r.summary.mailbox_ids = s.query_map([&r.summary.id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            }
            Ok(rows)
        })
    }
}
