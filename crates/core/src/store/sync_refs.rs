//! Lookups between local ids and the ids every computer shares (the
//! provider's own), for proposals synced between computers (0080).

use super::*;

impl Store {
    /// A message's account, provider id and Message-ID header.
    pub fn message_sync_ref(&self, id: &str) -> Result<Option<(String, String, Option<String>)>> {
        self.r(|c| {
            Ok(c.query_row("SELECT account_id, remote_id, message_id_hdr FROM messages WHERE id=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?)
        })
    }

    /// The message here with that provider id, or else that Message-ID header.
    pub fn message_by_ref(&self, account_id: &str, remote_id: &str, hdr: Option<&str>) -> Result<Option<String>> {
        self.r(|c| {
            let by_remote = c
                .query_row("SELECT id FROM messages WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?;
            if by_remote.is_some() {
                return Ok(by_remote);
            }
            let Some(h) = hdr else { return Ok(None) };
            Ok(c.query_row(
                "SELECT id FROM messages WHERE account_id=? AND message_id_hdr=? ORDER BY date DESC LIMIT 1",
                [account_id, h],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    pub fn message_thread(&self, id: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT thread_id FROM messages WHERE id=?", [id], |r| r.get(0)).optional()?))
    }

    /// The newest message in a thread, which stands for the thread.
    pub fn thread_latest_message(&self, thread_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM messages WHERE thread_id=? ORDER BY date DESC LIMIT 1", [thread_id], |r| r.get(0))
                .optional()?)
        })
    }

    pub fn mailbox_ref(&self, id: &str) -> Result<Option<(String, String)>> {
        self.r(|c| Ok(c.query_row("SELECT account_id, remote_id FROM mailboxes WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?))
    }

    pub fn mailbox_id_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM mailboxes WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?)
        })
    }

    pub fn calendar_ref(&self, id: &str) -> Result<Option<(String, String)>> {
        self.r(|c| Ok(c.query_row("SELECT account_id, remote_id FROM calendars WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?))
    }

    pub fn calendar_id_by_remote(&self, account_id: &str, remote_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM calendars WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?)
        })
    }

    /// An event's calendar and provider id.
    pub fn event_ref(&self, id: &str) -> Result<Option<(String, String)>> {
        self.r(|c| Ok(c.query_row("SELECT calendar_id, remote_id FROM events WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?))
    }

    pub fn event_id_by_remote(&self, calendar_id: &str, remote_id: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT id FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id], |r| r.get(0))
                .optional()?)
        })
    }

    /// A proposal that came from another computer, decided or not.
    pub fn insert_synced_proposal(&self, p: &Proposal, versions: &HashMap<String, i64>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO proposals (id, agent_id, created_at, status, action_json, rationale, source_threads_json, warnings_json, summary, versions_json, review_note, decided_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    p.id, p.agent_id, p.created_at.timestamp_millis(), proposal_status_str(p.status), to_json(&p.action), p.rationale,
                    to_json(&p.source_thread_ids), to_json(&p.warnings), p.summary, to_json(versions), p.review_note,
                    p.decided_at.map(|d| d.timestamp_millis())
                ],
            )?;
            Ok(())
        })
    }

    /// Takes another computer's decision as it was: status, note, time, and
    /// the action if it was edited before approval.
    pub fn set_synced_proposal_outcome(&self, p: &Proposal) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE proposals SET status=?, review_note=?, decided_at=?, action_json=? WHERE id=?",
                params![proposal_status_str(p.status), p.review_note, p.decided_at.map(|d| d.timestamp_millis()), to_json(&p.action), p.id],
            )?;
            Ok(())
        })
    }

    /// Proposals made since `since`, newest first.
    pub fn proposals_created_since(&self, since: DateTime<Utc>) -> Result<Vec<Proposal>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT p.*, a.name AS agent_name FROM proposals p LEFT JOIN agents a ON a.id=p.agent_id WHERE p.created_at>=? ORDER BY p.created_at DESC",
            )?;
            let rows = s.query_map([since.timestamp_millis()], Self::proposal_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(|x| x.0).collect())
        })
    }
}
