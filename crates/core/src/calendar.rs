//! Calendar queries: occurrence expansion and free-time search.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc};
use satchel_api::*;
use satchel_ical::ParsedEvent;

use crate::store::{StoredEvent, Store};

fn far_future() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2200, 1, 1, 0, 0, 0).unwrap()
}

/// The [start, end) span covered by a recurring series (or single event).
pub fn series_range(components: &[ParsedEvent]) -> (DateTime<Utc>, DateTime<Utc>) {
    let mut start = far_future();
    let mut end = Utc.timestamp_opt(0, 0).unwrap();
    for c in components {
        start = start.min(c.start_utc());
        let e = match &c.rrule {
            None => c.end_utc(),
            Some(rule) => {
                let upper = rule.to_ascii_uppercase();
                if upper.contains("UNTIL=") || upper.contains("COUNT=") {
                    let occ = satchel_ical::expand(c, &[], c.start_utc(), far_future());
                    occ.last().map(|o| o.1).unwrap_or_else(|| c.end_utc())
                } else {
                    far_future()
                }
            }
        };
        end = end.max(e);
    }
    if end < start {
        end = start;
    }
    (start, end)
}

pub struct ExpandedEvent {
    pub stored: StoredEvent,
    pub event: Event,
}

/// The owner's address for computing `my_status`.
fn owner_email(store: &Store, account_id: &str, cache: &mut HashMap<String, Option<String>>) -> Option<String> {
    cache
        .entry(account_id.to_string())
        .or_insert_with(|| store.get_account(account_id).ok().flatten().map(|a| a.email))
        .clone()
}

pub fn occurrences(store: &Store, from: DateTime<Utc>, to: DateTime<Utc>, calendar_ids: Option<&[String]>) -> anyhow::Result<Vec<Occurrence>> {
    let cals: HashMap<String, Calendar> = store.list_calendars(None)?.into_iter().map(|c| (c.cal.id.clone(), c.cal)).collect();
    let visible: Vec<String> = match calendar_ids {
        Some(ids) => ids.to_vec(),
        None => cals.values().filter(|c| c.visible).map(|c| c.id.clone()).collect(),
    };
    let stored = store.events_in_range(from, to, Some(&visible))?;
    let mut owners = HashMap::new();
    // Group resources by UID so separately-stored overrides join their master.
    type Resources = Vec<(StoredEvent, Vec<ParsedEvent>)>;
    let mut groups: HashMap<(String, String), Resources> = HashMap::new();
    for s in stored {
        let parsed = satchel_ical::parse(&s.ics).events;
        groups.entry((s.calendar_id.clone(), s.uid.clone())).or_default().push((s, parsed));
    }
    let mut out = vec![];
    for ((cal_id, _uid), resources) in groups {
        let color = cals.get(&cal_id).map(|c| c.color.clone()).unwrap_or_else(|| "#4f7cff".into());
        let mut master: Option<(StoredEvent, ParsedEvent)> = None;
        let mut overrides: Vec<(StoredEvent, ParsedEvent)> = vec![];
        for (s, comps) in resources {
            for c in comps {
                if c.recurrence_id.is_some() {
                    overrides.push((s.clone(), c));
                } else if master.is_none() {
                    master = Some((s.clone(), c));
                }
            }
        }
        let owner = master
            .as_ref()
            .map(|(s, _)| s.account_id.clone())
            .or_else(|| overrides.first().map(|(s, _)| s.account_id.clone()))
            .and_then(|a| owner_email(store, &a, &mut owners));
        match master {
            Some((ms, m)) => {
                let ov: Vec<ParsedEvent> = overrides.iter().map(|(_, o)| o.clone()).collect();
                for (start, end, idx) in satchel_ical::expand(&m, &ov, from, to) {
                    let (src_stored, src) = match idx {
                        Some(i) => (&overrides[i].0, &overrides[i].1),
                        None => (&ms, &m),
                    };
                    let mut ev = src.to_event(&cal_id, owner.as_deref());
                    ev.id = Some(src_stored.id.clone());
                    if idx.is_some() {
                        ev.rrule = m.rrule.clone();
                    }
                    if ev.status == EventStatus::Cancelled {
                        continue;
                    }
                    out.push(Occurrence { event: ev, start, end, color: color.clone() });
                }
            }
            None => {
                // Orphan overrides (master not visible to us): show them as-is.
                for (s, o) in overrides {
                    let (start, end) = (o.start_utc(), o.end_utc());
                    if end > from && start < to {
                        let mut ev = o.to_event(&cal_id, owner.as_deref());
                        ev.id = Some(s.id.clone());
                        out.push(Occurrence { event: ev, start, end, color: color.clone() });
                    }
                }
            }
        }
    }
    out.sort_by_key(|o| o.start);
    Ok(out)
}

/// The stored event as an API object (master component).
pub fn get_event(store: &Store, id: &str) -> anyhow::Result<Option<Event>> {
    let Some(s) = store.get_event(id)? else { return Ok(None) };
    let parsed = satchel_ical::parse(&s.ics);
    let owner = store.get_account(&s.account_id)?.map(|a| a.email);
    let Some(m) = parsed.events.iter().find(|e| e.recurrence_id.is_none()).or(parsed.events.first()) else {
        return Ok(None);
    };
    let mut e = m.to_event(&s.calendar_id, owner.as_deref());
    e.id = Some(s.id);
    Ok(Some(e))
}

pub fn free_slots(
    busy: &[Occurrence],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    duration: Duration,
    day_start: u32,
    day_end: u32,
    tz: chrono_tz::Tz,
) -> Vec<FreeSlot> {
    let mut intervals: Vec<(DateTime<Utc>, DateTime<Utc>)> = busy
        .iter()
        .filter(|o| !o.event.all_day)
        .filter(|o| o.event.my_status != Some(PartStat::Declined))
        .filter(|o| o.event.status != EventStatus::Cancelled)
        .map(|o| (o.start, o.end))
        .collect();
    intervals.sort();
    let mut out = vec![];
    let mut day = from.with_timezone(&tz).date_naive();
    let last_day = to.with_timezone(&tz).date_naive();
    while day <= last_day {
        let wd = day.weekday().num_days_from_monday();
        if wd < 5 {
            let ds = tz
                .from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(day_start.min(23), 0, 0).unwrap()))
                .earliest()
                .map(|d| d.with_timezone(&Utc));
            let de = tz
                .from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(day_end.min(23), 59, 0).unwrap()))
                .earliest()
                .map(|d| d.with_timezone(&Utc));
            if let (Some(mut cur), Some(end)) = (ds, de) {
                let end = if day_end >= 24 { end } else { tz.from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(day_end, 0, 0).unwrap())).earliest().map(|d| d.with_timezone(&Utc)).unwrap_or(end) };
                cur = cur.max(from);
                let end = end.min(to);
                for (bs, be) in &intervals {
                    if *be <= cur || *bs >= end {
                        continue;
                    }
                    if *bs > cur && *bs - cur >= duration {
                        out.push(FreeSlot { start: cur, end: *bs });
                    }
                    cur = cur.max(*be);
                }
                if end > cur && end - cur >= duration {
                    out.push(FreeSlot { start: cur, end });
                }
            }
        }
        day = day.succ_opt().unwrap();
    }
    out
}
