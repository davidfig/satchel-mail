//! Calendar writes.


use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use satchel_api::*;
use serde_json::json;

use crate::engine::{Caller, Engine};
use crate::error::{invalid, not_found};
use crate::store::new_id;

impl Engine {
    pub(crate) fn visible_calendar_ids(&self, caller: &Caller, requested: Option<Vec<String>>) -> Result<Vec<String>> {
        let allowed = self.allowed_accounts(caller);
        let cals = self.store.list_calendars(None)?;
        Ok(cals
            .into_iter()
            .filter(|c| allowed.as_ref().is_none_or(|ids| ids.contains(&c.cal.account_id)))
            .filter(|c| match &requested {
                Some(r) => r.contains(&c.cal.id),
                None => c.cal.visible,
            })
            .map(|c| c.cal.id)
            .collect())
    }

    pub(crate) fn validate_event(&self, e: &Event) -> Result<crate::store::StoredCalendar> {
        if e.title.trim().is_empty() {
            return Err(invalid("event needs a title"));
        }
        if e.end < e.start {
            return Err(invalid("event ends before it starts"));
        }
        if let Some(tz) = &e.timezone {
            tz.parse::<chrono_tz::Tz>().map_err(|_| invalid("unknown timezone"))?;
        }
        let cal = self.store.get_calendar(&e.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        if cal.cal.read_only {
            return Err(invalid("calendar is read-only"));
        }
        Ok(cal)
    }

    pub(crate) fn create_event(&self, mut e: Event, actor: &str) -> Result<Event> {
        let cal = self.validate_event(&e)?;
        let acc = self.store.get_account(&cal.cal.account_id)?.ok_or_else(|| not_found("account"))?;
        e.uid = Some(e.uid.clone().unwrap_or_else(satchel_ical::new_uid));
        if !e.attendees.is_empty() && e.organizer.is_none() {
            e.organizer = Some(Address { name: Some(acc.display_name.clone()), email: acc.email.clone() });
        }
        // Agents and older clients don't say: new timed events get the default alert.
        if e.alerts.is_none() {
            e.alerts = Some(self.settings().default_alert_minutes.filter(|_| !e.all_day).map(|m| Alert { minutes_before: m }).into_iter().collect());
        }
        let ics = satchel_ical::serialize(&e, &Default::default());
        let parsed = satchel_ical::parse(&ics);
        let range = crate::calendar::series_range(&parsed.events);
        let id = self.store.upsert_event(&cal.cal.id, &acc.id, &format!("local:{}", new_id()), None, e.uid.as_deref().unwrap(), &ics, &e.title, e.location.trim(), range, e.rrule.is_some(), true)?;
        self.enqueue(&acc.id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "event_created", &json!({ "title": e.title, "start": e.start }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: acc.id.clone() });
        e.id = Some(id);
        Ok(e)
    }

    pub(crate) fn update_event(&self, mut e: Event, actor: &str) -> Result<Event> {
        let id = e.id.clone().ok_or_else(|| invalid("event id required"))?;
        let stored = self.store.get_event(&id)?.ok_or_else(|| not_found("event"))?;
        if stored.calendar_id != e.calendar_id {
            return self.move_event(stored, e, actor);
        }
        self.validate_event(&e)?;
        let (ics, range) = rewrite_ics(&stored, &mut e);
        self.store.update_event_ics(&id, &ics, &e.title, e.location.trim(), range, e.rrule.is_some())?;
        self.enqueue(&stored.account_id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "event_updated", &json!({ "title": e.title, "start": e.start }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: stored.account_id.clone() });
        Ok(e)
    }

    /// Move an event to another calendar, possibly another account: a copy
    /// with the same UID goes in the new one and the old one is deleted
    /// silently. Servers treat that as a cancellation plus a new invitation,
    /// so events with guests can't move (0049).
    fn move_event(&self, stored: crate::store::StoredEvent, mut e: Event, actor: &str) -> Result<Event> {
        let to = self.validate_event(&e)?;
        let from = self.store.get_calendar(&stored.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        if from.cal.read_only {
            return Err(invalid("calendar is read-only"));
        }
        let had_guests = satchel_ical::parse(&stored.ics).events.iter().any(|x| !x.attendees.is_empty());
        if had_guests || !e.attendees.is_empty() {
            return Err(invalid("events with guests can't move to another calendar: their guests would get a cancellation and a new invitation"));
        }
        let (ics, range) = rewrite_ics(&stored, &mut e);
        let id = self.store.upsert_event(&to.cal.id, &to.cal.account_id, &format!("local:{}", new_id()), None, &stored.uid, &ics, &e.title, e.location.trim(), range, e.rrule.is_some(), true)?;
        self.store.delete_event(&stored.id)?;
        // The delete goes first: some servers (iCloud, Google's CalDAV) refuse
        // a UID that's still in another of the account's calendars.
        self.enqueue(&stored.account_id, "cal_delete", json!({ "calendar_remote_id": from.remote_id, "remote_id": stored.remote_id, "etag": stored.etag, "notify": false }), actor)?;
        self.enqueue(&to.cal.account_id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "event_moved", &json!({ "title": e.title, "from": from.cal.name, "to": to.cal.name }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: stored.account_id.clone() });
        if to.cal.account_id != stored.account_id {
            self.emit(CoreEvent::CalendarChanged { account_id: to.cal.account_id.clone() });
        }
        e.id = Some(id);
        Ok(e)
    }

    pub(crate) fn delete_event(&self, id: &str, actor: &str) -> Result<()> {
        let stored = self.store.get_event(id)?.ok_or_else(|| not_found("event"))?;
        let cal = self.store.get_calendar(&stored.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        if cal.cal.read_only {
            return Err(invalid("calendar is read-only"));
        }
        self.store.delete_event(id)?;
        self.enqueue(&stored.account_id, "cal_delete", json!({ "calendar_remote_id": cal.remote_id, "remote_id": stored.remote_id, "etag": stored.etag }), actor)?;
        self.store.audit(actor, "event_deleted", &json!({ "uid": stored.uid }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: stored.account_id });
        Ok(())
    }

    pub(crate) fn respond(&self, id: &str, response: PartStat, actor: &str) -> Result<()> {
        let stored = self.store.get_event(id)?.ok_or_else(|| not_found("event"))?;
        let acc = self.store.get_account(&stored.account_id)?.ok_or_else(|| not_found("account"))?;
        let parsed = satchel_ical::parse(&stored.ics);
        if !parsed.events.iter().any(|e| e.attendees.iter().any(|a| a.email.eq_ignore_ascii_case(&acc.email))) {
            return Err(invalid("you are not an attendee of this event"));
        }
        let ics = satchel_ical::set_partstat(&stored.ics, &acc.email, response);
        let reparsed = satchel_ical::parse(&ics);
        let range = crate::calendar::series_range(&reparsed.events);
        let title = reparsed.events.first().map(|e| e.summary.clone()).unwrap_or_default();
        let location = reparsed.events.first().map(|e| e.location.trim().to_string()).unwrap_or_default();
        self.store.update_event_ics(id, &ics, &title, &location, range, reparsed.events.iter().any(|e| e.rrule.is_some()))?;
        self.enqueue(&acc.id, "cal_put", json!({ "event_id": id }), actor)?;
        self.store.audit(actor, "invite_response", &json!({ "event": title, "response": satchel_ical::partstat_str(response) }))?;
        self.emit(CoreEvent::CalendarChanged { account_id: acc.id });
        Ok(())
    }

    pub(crate) fn list_calendars(&self, caller: &Caller) -> Result<Vec<Calendar>> {
        let allowed = self.allowed_accounts(caller);
        Ok(self.store.list_calendars(None)?.into_iter().map(|c| c.cal).filter(|c| allowed.as_ref().is_none_or(|ids| ids.contains(&c.account_id))).collect())
    }

    pub(crate) fn list_events(&self, caller: &Caller, from: DateTime<Utc>, to: DateTime<Utc>, calendar_ids: Option<Vec<String>>) -> Result<Vec<Occurrence>> {
        if to - from > Duration::days(400) {
            return Err(invalid("range too large (max 400 days)"));
        }
        let ids = self.visible_calendar_ids(caller, calendar_ids)?;
        crate::calendar::occurrences(&self.store, from, to, Some(&ids))
    }

    pub(crate) fn get_event(&self, caller: &Caller, event_id: &str) -> Result<Event> {
        let e = crate::calendar::get_event(&self.store, event_id)?.ok_or_else(|| not_found("event"))?;
        let cal = self.store.get_calendar(&e.calendar_id)?.ok_or_else(|| not_found("calendar"))?;
        self.check_account(caller, &cal.cal.account_id, "event")?;
        Ok(e)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn find_free_time(
        &self,
        caller: &Caller,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        duration_minutes: u32,
        day_start_hour: Option<u32>,
        day_end_hour: Option<u32>,
        timezone: Option<&str>,
    ) -> Result<Vec<FreeSlot>> {
        // The same bound as `list_events`: expanding years of recurrences
        // and walking every day of them is slow.
        if to - from > Duration::days(400) {
            return Err(invalid("range too large (max 400 days)"));
        }
        let tz: chrono_tz::Tz = timezone.unwrap_or("UTC").parse().map_err(|_| invalid("unknown timezone"))?;
        let ids = self.visible_calendar_ids(caller, None)?;
        let busy = crate::calendar::occurrences(&self.store, from, to, Some(&ids))?;
        Ok(crate::calendar::free_slots(
            &busy,
            from,
            to,
            Duration::minutes(duration_minutes.max(5) as i64),
            day_start_hour.unwrap_or(9),
            day_end_hour.unwrap_or(17),
            tz,
        ))
    }
}

/// `e` as iCalendar, keeping what the editor doesn't show from the stored
/// copy: its UID, cancelled dates, changed occurrences and alarms, with the
/// sequence bumped. Also returns the span the series covers.
///
/// Alarms set elsewhere survive the edit untouched (with properties we don't
/// model) unless `e.alerts` changes them; then new ones replace those that
/// show, and email alarms stay (0056).
fn rewrite_ics(stored: &crate::store::StoredEvent, e: &mut Event) -> (String, (DateTime<Utc>, DateTime<Utc>)) {
    let old = satchel_ical::parse(&stored.ics);
    let master = old.events.iter().find(|x| x.recurrence_id.is_none());
    let exdates: Vec<DateTime<Utc>> = master.map(|m| m.exdates.iter().map(|x| x.to_utc()).collect()).unwrap_or_default();
    let sequence = master.map(|m| m.sequence + 1).unwrap_or(1);
    e.uid = Some(stored.uid.clone());
    let overrides = if e.rrule.is_some() { satchel_ical::override_blocks(&stored.ics) } else { vec![] };
    let old_alerts = master.map(|m| m.alerts()).unwrap_or_default();
    let unchanged = e.alerts.as_ref().is_none_or(|a| *a == old_alerts);
    let alarms: Vec<String> = satchel_ical::alarm_blocks(&stored.ics).into_iter().filter(|(_, shows)| unchanged || !shows).map(|(b, _)| b).collect();
    if unchanged {
        e.alerts = None;
    }
    let ics = satchel_ical::serialize(e, &satchel_ical::SerializeOpts { sequence, exdates: &exdates, extra_components: &overrides, alarms: &alarms, ..Default::default() });
    let parsed = satchel_ical::parse(&ics);
    e.alerts = Some(parsed.events.iter().find(|x| x.recurrence_id.is_none()).map(|m| m.alerts()).unwrap_or_default());
    let range = crate::calendar::series_range(&parsed.events);
    (ics, range)
}
