//! Calendars and events.

use super::*;

impl Store {
    pub fn upsert_calendar(&self, account_id: &str, remote_id: &str, name: &str, color: Option<&str>, read_only: bool) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM calendars WHERE account_id=? AND remote_id=?", [account_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE calendars SET name=?, read_only=?, color=COALESCE(?, color) WHERE id=?",
                    params![name, read_only, color, id],
                )?;
                return Ok(id);
            }
            let n: i64 = c.query_row("SELECT COUNT(*) FROM calendars", [], |r| r.get(0))?;
            let id = new_id();
            let col = color.map(|s| s.to_string()).unwrap_or_else(|| PALETTE[(n as usize + 2) % PALETTE.len()].to_string());
            c.execute(
                "INSERT INTO calendars (id, account_id, remote_id, name, color, read_only) VALUES (?,?,?,?,?,?)",
                params![id, account_id, remote_id, name, col, read_only],
            )?;
            Ok(id)
        })
    }

    fn calendar_row(r: &Row) -> rusqlite::Result<StoredCalendar> {
        Ok(StoredCalendar {
            cal: Calendar {
                id: r.get("id")?,
                account_id: r.get("account_id")?,
                name: r.get("name")?,
                color: r.get("color")?,
                read_only: r.get("read_only")?,
                visible: r.get("visible")?,
            },
            remote_id: r.get("remote_id")?,
            sync_token: r.get("sync_token")?,
        })
    }

    pub fn list_calendars(&self, account_id: Option<&str>) -> Result<Vec<StoredCalendar>> {
        self.r(|c| {
            let rows = match account_id {
                Some(a) => {
                    let mut s = c.prepare("SELECT * FROM calendars WHERE account_id=? ORDER BY name")?;
                    s.query_map([a], Self::calendar_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
                None => {
                    let mut s = c.prepare("SELECT * FROM calendars ORDER BY account_id, name")?;
                    s.query_map([], Self::calendar_row)?.collect::<rusqlite::Result<Vec<_>>>()?
                }
            };
            Ok(rows)
        })
    }

    pub fn get_calendar(&self, id: &str) -> Result<Option<StoredCalendar>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM calendars WHERE id=?", [id], Self::calendar_row).optional()?))
    }

    pub fn set_calendar_visible(&self, id: &str, visible: bool) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE calendars SET visible=? WHERE id=?", params![visible, id])?;
            Ok(())
        })
    }

    pub fn set_calendar_token(&self, id: &str, token: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE calendars SET sync_token=? WHERE id=?", params![token, id])?;
            Ok(())
        })
    }

    pub fn prune_calendars(&self, account_id: &str, keep_remote: &[String]) -> Result<()> {
        let all = self.list_calendars(Some(account_id))?;
        self.w(|c| {
            for cal in all {
                if !keep_remote.contains(&cal.remote_id) {
                    c.execute("DELETE FROM calendars WHERE id=?", [&cal.cal.id])?;
                }
            }
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_event(
        &self,
        calendar_id: &str,
        account_id: &str,
        remote_id: &str,
        etag: Option<&str>,
        uid: &str,
        ics: &str,
        title: &str,
        range: (DateTime<Utc>, DateTime<Utc>),
        recurring: bool,
        pending: bool,
    ) -> Result<String> {
        self.w(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                c.execute(
                    "UPDATE events SET etag=?, uid=?, ics=?, title=?, range_start=?, range_end=?, is_recurring=?, pending=?, version=version+1 WHERE id=?",
                    params![etag, uid, ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, pending, id],
                )?;
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO events (id, calendar_id, account_id, remote_id, etag, uid, ics, title, range_start, range_end, is_recurring, pending)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                params![id, calendar_id, account_id, remote_id, etag, uid, ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, pending],
            )?;
            Ok(id)
        })
    }

    fn event_row(r: &Row) -> rusqlite::Result<StoredEvent> {
        Ok(StoredEvent {
            id: r.get("id")?,
            calendar_id: r.get("calendar_id")?,
            account_id: r.get("account_id")?,
            remote_id: r.get("remote_id")?,
            etag: r.get("etag")?,
            uid: r.get("uid")?,
            ics: r.get("ics")?,
            version: r.get("version")?,
            pending: r.get("pending")?,
        })
    }

    pub fn get_event(&self, id: &str) -> Result<Option<StoredEvent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM events WHERE id=?", [id], Self::event_row).optional()?))
    }

    pub fn event_by_remote(&self, calendar_id: &str, remote_id: &str) -> Result<Option<StoredEvent>> {
        self.r(|c| {
            Ok(c.query_row("SELECT * FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id], Self::event_row)
                .optional()?)
        })
    }

    pub fn events_by_uid(&self, account_id: &str, uid: &str) -> Result<Vec<StoredEvent>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM events WHERE account_id=? AND uid=?")?;
            let rows = s.query_map([account_id, uid], Self::event_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn events_in_range(&self, from: DateTime<Utc>, to: DateTime<Utc>, calendar_ids: Option<&[String]>) -> Result<Vec<StoredEvent>> {
        self.r(|c| {
            let mut sql = String::from("SELECT * FROM events WHERE range_start<? AND range_end>?");
            let mut p: Vec<rusqlite::types::Value> =
                vec![to.timestamp_millis().into(), from.timestamp_millis().into()];
            if let Some(ids) = calendar_ids {
                if ids.is_empty() {
                    return Ok(vec![]);
                }
                sql.push_str(&format!(" AND calendar_id IN ({})", vec!["?"; ids.len()].join(",")));
                for i in ids {
                    p.push(i.clone().into());
                }
            }
            let mut s = c.prepare(&sql)?;
            let rows = s.query_map(params_from_iter(p.iter()), Self::event_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn event_remote_ids(&self, calendar_id: &str) -> Result<HashMap<String, (String, Option<String>)>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT remote_id, id, etag FROM events WHERE calendar_id=? AND pending=0")?;
            let rows = s
                .query_map([calendar_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok(rows)
        })
    }

    pub fn set_event_remote(&self, id: &str, remote_id: &str, etag: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE events SET remote_id=?, etag=?, pending=0 WHERE id=?",
                params![remote_id, etag, id],
            )?;
            Ok(())
        })
    }

    pub fn update_event_ics(&self, id: &str, ics: &str, title: &str, range: (DateTime<Utc>, DateTime<Utc>), recurring: bool) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE events SET ics=?, title=?, range_start=?, range_end=?, is_recurring=?, pending=1, version=version+1 WHERE id=?",
                params![ics, title, range.0.timestamp_millis(), range.1.timestamp_millis(), recurring, id],
            )?;
            Ok(())
        })
    }

    /// Returns whether the event existed (so concurrent deletes push one op).
    pub fn delete_event(&self, id: &str) -> Result<bool> {
        self.w(|c| Ok(c.execute("DELETE FROM events WHERE id=?", [id])? > 0))
    }

    pub fn delete_event_by_remote(&self, calendar_id: &str, remote_id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM events WHERE calendar_id=? AND remote_id=?", [calendar_id, remote_id])?;
            Ok(())
        })
    }
}
