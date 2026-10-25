//! Per-sender spam rules.

use super::*;

impl Store {
    pub fn set_sender_rule(&self, email: &str, verdict: Option<SenderVerdict>) -> Result<()> {
        let email = email.trim().to_lowercase();
        self.w(|c| {
            match verdict {
                Some(v) => c.execute(
                    "INSERT INTO sender_rules (email, verdict, created_at) VALUES (?,?,?)
                     ON CONFLICT(email) DO UPDATE SET verdict=excluded.verdict, created_at=excluded.created_at",
                    params![email, verdict_str(v), now_ms()],
                )?,
                None => c.execute("DELETE FROM sender_rules WHERE email=?", [email])?,
            };
            Ok(())
        })
    }

    pub fn sender_rule(&self, email: &str) -> Result<Option<SenderVerdict>> {
        self.r(|c| {
            let v: Option<String> = c.query_row("SELECT verdict FROM sender_rules WHERE email=?", [email.trim().to_lowercase()], |r| r.get(0)).optional()?;
            Ok(v.as_deref().map(parse_verdict))
        })
    }

    pub fn list_sender_rules(&self) -> Result<Vec<SenderRule>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT email, verdict, created_at FROM sender_rules ORDER BY created_at DESC")?;
            let rows = s
                .query_map([], |r| Ok(SenderRule { email: r.get(0)?, verdict: parse_verdict(&r.get::<_, String>(1)?), created_at: ms_to_dt(r.get(2)?) }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Recent messages with attachments from senders marked as spam: the ones
    /// that may carry an invitation the server put on the calendar.
    pub fn spam_rule_messages_with_attachments(&self, account_id: &str, since: DateTime<Utc>) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT m.id FROM sender_rules r JOIN messages m ON m.account_id=?1 AND m.from_email = r.email COLLATE NOCASE
                 WHERE r.verdict='spam' AND m.has_attachments=1 AND m.date >= ?2",
            )?;
            let rows = s.query_map(params![account_id, since.timestamp_millis()], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }

    /// Messages a sender rule says are in the wrong place: spam senders still
    /// in the inbox, and not-spam senders in Junk.
    pub fn misplaced_by_rules(&self, account_id: &str) -> Result<Vec<(String, SenderVerdict)>> {
        self.r(|c| {
            let mut s = c.prepare(
                "SELECT m.id, r.verdict FROM sender_rules r JOIN messages m ON m.account_id=?1 AND m.from_email = r.email COLLATE NOCASE
                 WHERE EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id=mm.mailbox_id
                               WHERE mm.message_id=m.id AND mb.role = CASE r.verdict WHEN 'spam' THEN 'inbox' ELSE 'junk' END)",
            )?;
            let rows = s
                .query_map([account_id], |r| Ok((r.get::<_, String>(0)?, parse_verdict(&r.get::<_, String>(1)?))))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
