//! Calendar alerts that have gone off (0056).

use super::*;

/// Where an alert stands once it has gone off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    Due,
    Snoozed,
    Dismissed,
}

impl AlertState {
    fn as_str(self) -> &'static str {
        match self {
            AlertState::Due => "due",
            AlertState::Snoozed => "snoozed",
            AlertState::Dismissed => "dismissed",
        }
    }
}

/// One alert of one occurrence of an event.
#[derive(Debug, Clone)]
pub struct AlertRow {
    pub id: String,
    pub event_id: String,
    pub occurrence: DateTime<Utc>,
    pub occurrence_end: DateTime<Utc>,
    pub minutes_before: i32,
    /// When it went off, or when a snooze ends.
    pub due_at: DateTime<Utc>,
    pub state: AlertState,
}

impl Store {
    fn alert_row(r: &Row) -> rusqlite::Result<AlertRow> {
        let state: String = r.get("state")?;
        Ok(AlertRow {
            id: r.get("id")?,
            event_id: r.get("event_id")?,
            occurrence: ms_to_dt(r.get("occurrence")?),
            occurrence_end: ms_to_dt(r.get("occurrence_end")?),
            minutes_before: r.get("minutes_before")?,
            due_at: ms_to_dt(r.get("due_at")?),
            state: match state.as_str() {
                "snoozed" => AlertState::Snoozed,
                "dismissed" => AlertState::Dismissed,
                _ => AlertState::Due,
            },
        })
    }

    /// Every recorded alert, soonest due first.
    pub fn list_alert_rows(&self) -> Result<Vec<AlertRow>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM calendar_alerts ORDER BY due_at")?;
            let rows = s.query_map([], Self::alert_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_alert_row(&self, id: &str) -> Result<Option<AlertRow>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM calendar_alerts WHERE id=?", [id], Self::alert_row).optional()?))
    }

    /// Record an alert; false if it already was.
    pub fn insert_alert(&self, a: &AlertRow) -> Result<bool> {
        self.w(|c| {
            let n = c.execute(
                "INSERT INTO calendar_alerts (id, event_id, occurrence, occurrence_end, minutes_before, due_at, state) VALUES (?,?,?,?,?,?,?)
                 ON CONFLICT DO NOTHING",
                params![a.id, a.event_id, a.occurrence.timestamp_millis(), a.occurrence_end.timestamp_millis(), a.minutes_before, a.due_at.timestamp_millis(), a.state.as_str()],
            )?;
            Ok(n > 0)
        })
    }

    /// Returns whether the alert exists.
    pub fn set_alert_state(&self, id: &str, state: AlertState, due_at: Option<DateTime<Utc>>) -> Result<bool> {
        self.w(|c| {
            Ok(c.execute(
                "UPDATE calendar_alerts SET state=?, due_at=COALESCE(?, due_at) WHERE id=?",
                params![state.as_str(), due_at.map(|d| d.timestamp_millis()), id],
            )? > 0)
        })
    }

    /// Dismiss every alert that's showing or snoozed. Returns their ids.
    pub fn dismiss_all_alerts(&self) -> Result<Vec<String>> {
        self.w(|c| {
            let mut s = c.prepare("UPDATE calendar_alerts SET state='dismissed' WHERE state<>'dismissed' RETURNING id")?;
            let ids = s.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(ids)
        })
    }

    pub fn delete_alerts(&self, ids: &[String]) -> Result<()> {
        self.w(|c| {
            for id in ids {
                c.execute("DELETE FROM calendar_alerts WHERE id=?", [id])?;
            }
            Ok(())
        })
    }

    /// Forget alerts of occurrences that ended before `before` and were due
    /// before it too: they can't go off again.
    pub fn prune_alerts(&self, before: DateTime<Utc>) -> Result<()> {
        let t = before.timestamp_millis();
        self.w(|c| {
            c.execute("DELETE FROM calendar_alerts WHERE occurrence_end<? AND due_at<?", [t, t])?;
            Ok(())
        })
    }
}
