//! Calendar alerts: finding the ones that are due or coming up, and snoozing
//! or dismissing them (0056, 0057). The worker that runs
//! [`Engine::alerts_pass`] is in `engine/workers.rs`.
//!
//! An alert's id is known before it goes off (see [`alert_id`]), so a host
//! that hands upcoming alerts to the OS can act on them by id even when the
//! core hasn't recorded them yet (the app was quit when the OS showed one).

use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use satchel_api::*;

use crate::engine::Engine;
use crate::error::{invalid, not_found};
use crate::store::{AlertRow, AlertState, Store};

/// How far back occurrences are looked at: alerts after the start, and
/// alerts kept while their event goes on.
const LOOKBACK: Duration = Duration::days(1);
/// How far ahead: the longest alert Google allows is 4 weeks before.
const LOOKAHEAD: Duration = Duration::days(28);
/// An alert that came due while nothing was running still shows this late.
const LATE: Duration = Duration::hours(1);
/// The longest snooze.
const MAX_SNOOZE: Duration = Duration::days(7);
/// The furthest ahead `upcoming_alerts` looks.
const MAX_UPCOMING: Duration = Duration::days(14);

/// One alert of one occurrence: `{event id}:{occurrence start, ms}:{minutes before}`.
pub fn alert_id(event_id: &str, occurrence: DateTime<Utc>, minutes_before: i32) -> String {
    format!("{event_id}:{}:{minutes_before}", occurrence.timestamp_millis())
}

/// The event id, occurrence start and minutes of an [`alert_id`].
pub fn parse_alert_id(id: &str) -> Option<(String, DateTime<Utc>, i32)> {
    let mut parts = id.rsplitn(3, ':');
    let minutes = parts.next()?.parse().ok()?;
    let start = Utc.timestamp_millis_opt(parts.next()?.parse().ok()?).single()?;
    let event = parts.next().filter(|e| !e.is_empty())?;
    Some((event.to_string(), start, minutes))
}

/// An occurrence's span on the clock. All-day events are stored as UTC
/// midnights; their alerts count from midnight where the host is.
fn span(o: &Occurrence) -> (DateTime<Utc>, DateTime<Utc>) {
    if !o.event.all_day {
        return (o.start, o.end);
    }
    let local = |t: DateTime<Utc>| Local.from_local_datetime(&t.date_naive().and_time(NaiveTime::MIN)).earliest().map(|d| d.with_timezone(&Utc)).unwrap_or(t);
    (local(o.start), local(o.end))
}

/// The alerts an occurrence has, with when each goes off.
fn alert_times(o: &Occurrence) -> Vec<(i32, DateTime<Utc>)> {
    // Declined: you're not going.
    if o.event.my_status == Some(PartStat::Declined) {
        return vec![];
    }
    let (start, _) = span(o);
    let mut seen = HashSet::new();
    o.event
        .alerts
        .iter()
        .flatten()
        .filter(|a| seen.insert(a.minutes_before))
        .filter_map(|a| Some((a.minutes_before, start.checked_sub_signed(Duration::minutes(a.minutes_before.into()))?)))
        .collect()
}

fn to_api(row: &AlertRow, o: &Occurrence) -> CalendarAlert {
    CalendarAlert {
        id: row.id.clone(),
        event_id: row.event_id.clone(),
        calendar_id: o.event.calendar_id.clone(),
        title: o.event.title.clone(),
        location: o.event.location.clone(),
        start: o.start,
        end: o.end,
        all_day: o.event.all_day,
        color: o.color.clone(),
        minutes_before: row.minutes_before,
        due: row.due_at,
        snoozed: row.state == AlertState::Snoozed,
    }
}

/// An alert as it stands before it's recorded.
fn row(event_id: &str, o: &Occurrence, minutes: i32, due_at: DateTime<Utc>, state: AlertState) -> AlertRow {
    AlertRow { id: alert_id(event_id, o.start, minutes), event_id: event_id.to_string(), occurrence: o.start, occurrence_end: o.end, minutes_before: minutes, due_at, state }
}

/// The occurrence of `event_id` that starts at `start`, as it is now.
fn occurrence_of(store: &Store, event_id: &str, start: DateTime<Utc>) -> Result<Option<Occurrence>> {
    let Some(ev) = store.get_event(event_id)? else { return Ok(None) };
    let around = Duration::minutes(1);
    let found = super::occurrences(store, start - around, start + around, Some(&[ev.calendar_id]))?;
    Ok(found.into_iter().find(|o| o.event.id.as_deref() == Some(event_id) && o.start == start))
}

impl Engine {
    fn visible_occurrences(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Occurrence>> {
        let visible: Vec<String> = self.store.list_calendars(None)?.into_iter().filter(|c| c.cal.visible).map(|c| c.cal.id).collect();
        super::occurrences(&self.store, from, to, Some(&visible))
    }

    /// Record the alerts that are due at `now` and return them, with when
    /// the next one is. A recorded alert's `due` is when it was meant to go
    /// off (or a snooze's end), so it matches what `upcoming_alerts` said.
    pub fn alerts_pass(&self, now: DateTime<Utc>) -> Result<(Vec<CalendarAlert>, Option<DateTime<Utc>>)> {
        if !self.settings().calendar_alerts {
            return Ok((vec![], None));
        }
        let occurrences = self.visible_occurrences(now - LOOKBACK, now + LOOKAHEAD)?;
        let rows = self.store.list_alert_rows()?;
        let recorded: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let mut current = HashSet::new();
        let mut due = vec![];
        let mut next: Option<DateTime<Utc>> = None;
        for o in &occurrences {
            let Some(event_id) = o.event.id.as_deref() else { continue };
            for (minutes, at) in alert_times(o) {
                let r = row(event_id, o, minutes, at, AlertState::Due);
                current.insert(r.id.clone());
                if at > now {
                    next = Some(next.map_or(at, |n| n.min(at)));
                } else if at > now - LATE && span(o).1 > now && !recorded.contains(r.id.as_str()) && self.store.insert_alert(&r)? {
                    due.push(to_api(&r, o));
                }
            }
        }
        // Alerts whose event moved, lost the alert or went away stop showing;
        // dismissed ones stay recorded, so they can't go off again.
        let mut gone = vec![];
        for r in &rows {
            let in_window = r.occurrence_end > now - LOOKBACK;
            if r.state != AlertState::Dismissed && in_window && !current.contains(&r.id) {
                gone.push(r.id.clone());
                continue;
            }
            if r.state != AlertState::Snoozed {
                continue;
            }
            if r.due_at > now {
                next = Some(next.map_or(r.due_at, |n| n.min(r.due_at)));
            } else if let Some(o) = occurrence_of(&self.store, &r.event_id, r.occurrence)? {
                self.store.set_alert_state(&r.id, AlertState::Due, None)?;
                due.push(to_api(&AlertRow { state: AlertState::Due, ..r.clone() }, &o));
            } else {
                gone.push(r.id.clone());
            }
        }
        self.store.delete_alerts(&gone)?;
        self.store.prune_alerts(now - LOOKBACK)?;
        if !gone.is_empty() && due.is_empty() {
            self.emit(CoreEvent::AlertsChanged { due: vec![], closed: gone });
        }
        Ok((due, next))
    }

    /// Alerts that will go off after `now` and up to `until` (at most two
    /// weeks ahead), soonest first: ones not recorded yet, and snoozed ones
    /// (`snoozed`, with `due` the snooze's end). For hosts that schedule
    /// alerts with the OS, so they show even while the app is closed (0057).
    pub(crate) fn upcoming_alerts(&self, now: DateTime<Utc>, until: DateTime<Utc>) -> Result<Vec<CalendarAlert>> {
        if !self.settings().calendar_alerts {
            return Ok(vec![]);
        }
        let until = until.min(now + MAX_UPCOMING);
        let rows = self.store.list_alert_rows()?;
        let recorded: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let mut out = vec![];
        // Occurrences whose alerts go off by `until`: they start up to the
        // longest alert after it.
        for o in self.visible_occurrences(now - LOOKBACK, until + LOOKAHEAD)? {
            let Some(event_id) = o.event.id.as_deref() else { continue };
            for (minutes, at) in alert_times(&o) {
                let r = row(event_id, &o, minutes, at, AlertState::Due);
                if at > now && at <= until && !recorded.contains(r.id.as_str()) {
                    out.push(to_api(&r, &o));
                }
            }
        }
        for r in rows.iter().filter(|r| r.state == AlertState::Snoozed && r.due_at > now && r.due_at <= until) {
            if let Some(o) = occurrence_of(&self.store, &r.event_id, r.occurrence)? {
                out.push(to_api(r, &o));
            }
        }
        out.sort_by_key(|a| a.due);
        Ok(out)
    }

    /// Alerts that have gone off and aren't dismissed, soonest first.
    pub(crate) fn list_alerts(&self) -> Result<Vec<CalendarAlert>> {
        let mut out = vec![];
        for r in self.store.list_alert_rows()?.into_iter().filter(|r| r.state != AlertState::Dismissed) {
            if let Some(o) = occurrence_of(&self.store, &r.event_id, r.occurrence)? {
                out.push(to_api(&r, &o));
            }
        }
        Ok(out)
    }

    /// Put an alert in `state`, recording it first if it hasn't gone off
    /// here yet (the OS showed it while the app was closed).
    fn set_alert(&self, id: &str, state: AlertState, due_at: Option<DateTime<Utc>>) -> Result<()> {
        if self.store.set_alert_state(id, state, due_at)? {
            return Ok(());
        }
        let (event_id, start, minutes) = parse_alert_id(id).ok_or_else(|| not_found("alert"))?;
        let o = occurrence_of(&self.store, &event_id, start)?.ok_or_else(|| not_found("alert"))?;
        let at = alert_times(&o).into_iter().find(|(m, _)| *m == minutes).ok_or_else(|| not_found("alert"))?.1;
        self.store.insert_alert(&row(&event_id, &o, minutes, due_at.unwrap_or(at), state))?;
        Ok(())
    }

    pub(crate) fn snooze_alert(&self, id: &str, until: DateTime<Utc>) -> Result<()> {
        let now = Utc::now();
        if until <= now || until > now + MAX_SNOOZE {
            return Err(invalid("snooze for up to a week"));
        }
        self.set_alert(id, AlertState::Snoozed, Some(until))?;
        self.alerts_changed(vec![id.to_string()]);
        Ok(())
    }

    pub(crate) fn dismiss_alert(&self, id: Option<&str>) -> Result<()> {
        let closed = match id {
            Some(id) => {
                self.set_alert(id, AlertState::Dismissed, None)?;
                vec![id.to_string()]
            }
            None => self.store.dismiss_all_alerts()?,
        };
        self.alerts_changed(closed);
        Ok(())
    }

    fn alerts_changed(&self, closed: Vec<String>) {
        self.emit(CoreEvent::AlertsChanged { due: vec![], closed });
        self.alerts_wake.notify_one();
    }
}
