//! People you correspond with: suggestions and the person view.

use super::*;

impl Store {
    pub fn known_senders(&self, prefix: &str, limit: u32) -> Result<Vec<Address>> {
        self.r(|c| {
            let like = format!("{}%", prefix.to_lowercase());
            let mut s = c.prepare(
                "SELECT from_name, from_email, COUNT(*) n FROM messages WHERE from_email IS NOT NULL AND (lower(from_email) LIKE ?1 OR lower(from_name) LIKE ?1)
                 GROUP BY lower(from_email) ORDER BY n DESC LIMIT ?2",
            )?;
            let rows = s
                .query_map(params![like, limit], |r| Ok(Address { name: r.get(0)?, email: r.get(1)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Message counts, dates, names and events for one person's addresses.
    /// `threads` is left at 0 for the caller to fill from a `with:` count.
    pub fn person_summary(&self, emails: &[String]) -> Result<PersonSummary> {
        let emails: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        let mut out = PersonSummary::default();
        if emails.is_empty() {
            return Ok(out);
        }
        let ph = vec!["?"; emails.len()].join(",");
        self.r(move |c| {
            let (received, attachments, first, last): (u32, u32, Option<i64>, Option<i64>) = c.query_row(
                &format!("SELECT COUNT(*), COALESCE(SUM(has_attachments),0), MIN(date), MAX(date) FROM messages WHERE draft=0 AND lower(from_email) IN ({ph})"),
                params_from_iter(&emails),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            // Mail you sent them: from one of your accounts, with them on To, Cc or Bcc.
            let (sent, sfirst, slast): (u32, Option<i64>, Option<i64>) = c.query_row(
                &format!("SELECT COUNT(DISTINCT id), MIN(date), MAX(date) FROM ({SENT_TO}) WHERE e IN ({ph})"),
                params_from_iter(&emails),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            out.received = received;
            out.attachments = attachments;
            out.sent = sent;
            out.first_date = [first, sfirst].into_iter().flatten().min().map(ms_to_dt);
            out.last_date = [last, slast].into_iter().flatten().max().map(ms_to_dt);

            // Names: the one each address signs with most often, falling back to how you address them.
            let mut names = c.prepare(&format!(
                "SELECT lower(from_email), from_name, COUNT(*) n FROM messages WHERE lower(from_email) IN ({ph}) AND from_name IS NOT NULL AND from_name<>''
                 GROUP BY lower(from_email), from_name ORDER BY n DESC"
            ))?;
            let mut best: Vec<(String, String, u32)> = vec![];
            for r in names.query_map(params_from_iter(&emails), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?)))? {
                let (e, n, k) = r?;
                if !best.iter().any(|(x, _, _)| *x == e) {
                    best.push((e, n, k));
                }
            }
            if best.len() < emails.len() {
                let mut to_names = c.prepare(&format!(
                    "SELECT lower(json_extract(j.value,'$.email')) e, json_extract(j.value,'$.name') n, COUNT(*) k FROM messages m, json_each(m.to_json) j
                     WHERE e IN ({ph}) AND n IS NOT NULL AND n<>'' GROUP BY e, n ORDER BY k DESC"
                ))?;
                for r in to_names.query_map(params_from_iter(&emails), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?)))? {
                    let (e, n, k) = r?;
                    if !best.iter().any(|(x, _, _)| *x == e) {
                        best.push((e, n, k));
                    }
                }
            }
            out.addresses = emails
                .iter()
                .map(|e| Address { name: best.iter().find(|(x, _, _)| x == e).map(|(_, n, _)| n.clone()), email: e.clone() })
                .collect();
            out.name = best.iter().max_by_key(|(_, _, k)| *k).map(|(_, n, _)| n.clone());

            // Events: the address appears as ORGANIZER or ATTENDEE (unfolded, lowercased ICS).
            // The same invitation can sit in several of your calendars; show it once.
            let ics = "replace(replace(lower(ics), char(13)||char(10)||' ', ''), char(10)||' ', '')";
            let conds = vec![format!("instr({ics}, ?)>0"); emails.len()].join(" OR ");
            let mut ev = c.prepare(&format!(
                "SELECT MIN(e.id), e.title, e.range_start, e.is_recurring FROM events e
                 WHERE {conds} GROUP BY e.uid ORDER BY e.range_start DESC LIMIT 20"
            ))?;
            out.events = ev
                .query_map(params_from_iter(emails.iter().map(|e| format!("mailto:{e}"))), |r| {
                    Ok(PersonEvent { event_id: r.get(0)?, title: r.get(1)?, start: ms_to_dt(r.get(2)?), recurring: r.get(3)? })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(out)
        })
    }

    /// Addresses the user has written to (or replied to), with the names
    /// they appear under. Used to spot impersonation. Only mail in a Sent
    /// folder counts as written by the user.
    pub fn correspondents(&self) -> Result<Vec<(String, Option<String>)>> {
        self.r(|c| {
            let mut s = c.prepare(
                "WITH sent AS (SELECT m.to_json, m.cc_json FROM messages m JOIN accounts a ON a.id = m.account_id AND m.from_email = a.email COLLATE NOCASE
                               WHERE EXISTS (SELECT 1 FROM message_mailboxes mm JOIN mailboxes mb ON mb.id = mm.mailbox_id
                                             WHERE mm.message_id = m.id AND mb.role = 'sent'))
                 SELECT lower(json_extract(j.value, '$.email')), json_extract(j.value, '$.name') FROM sent, json_each(sent.to_json) j
                 UNION
                 SELECT lower(json_extract(j.value, '$.email')), json_extract(j.value, '$.name') FROM sent, json_each(sent.cc_json) j
                 UNION
                 SELECT lower(m.from_email), m.from_name FROM messages m WHERE m.answered = 1 AND m.from_email IS NOT NULL",
            )?;
            let rows = s
                .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get(1)?)))?
                .filter_map(|r| match r {
                    Ok((Some(e), n)) => Some(Ok((e, n))),
                    Ok((None, _)) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
