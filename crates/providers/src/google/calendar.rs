//! Google Calendar.


use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, RemoteCalendar, SyncCtx};
use serde_json::{Value, json};

use crate::oauth::ApiClient;

use super::api_client;

const GCAL: &str = "https://www.googleapis.com/calendar/v3";

pub struct GoogleCalendarProvider {
    api: ApiClient,
    owner: String,
}

fn gdate(v: &Value) -> Option<(DateTime<Utc>, bool, Option<String>)> {
    if let Some(dt) = v["dateTime"].as_str() {
        let d = DateTime::parse_from_rfc3339(dt).ok()?.with_timezone(&Utc);
        return Some((d, false, v["timeZone"].as_str().map(String::from)));
    }
    let d = chrono::NaiveDate::parse_from_str(v["date"].as_str()?, "%Y-%m-%d").ok()?;
    Some((Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?), true, None))
}

fn partstat_from_google(s: &str) -> PartStat {
    match s {
        "accepted" => PartStat::Accepted,
        "declined" => PartStat::Declined,
        "tentative" => PartStat::Tentative,
        _ => PartStat::NeedsAction,
    }
}

fn partstat_to_google(p: PartStat) -> &'static str {
    match p {
        PartStat::Accepted => "accepted",
        PartStat::Declined => "declined",
        PartStat::Tentative => "tentative",
        PartStat::NeedsAction => "needsAction",
    }
}

/// Google event JSON -> a VCALENDAR (single VEVENT; overrides carry RECURRENCE-ID).
pub fn google_to_ics(item: &Value, calendar_id: &str) -> Option<(String, String)> {
    // A deleted occurrence of a series comes with only its original start.
    let cancelled_instance = item["status"] == "cancelled" && item["recurringEventId"].is_string();
    let (start, all_day, tz) = gdate(&item["start"]).or_else(|| cancelled_instance.then(|| gdate(&item["originalStartTime"])).flatten())?;
    let (end, _, _) = gdate(&item["end"]).unwrap_or((start, all_day, None));
    let uid = item["iCalUID"].as_str().or(item["id"].as_str())?.to_string();
    let rrule = item["recurrence"].as_array().and_then(|r| r.iter().filter_map(Value::as_str).find(|l| l.starts_with("RRULE:")).map(|l| l.trim_start_matches("RRULE:").to_string()));
    let status = match item["status"].as_str() {
        Some("tentative") => EventStatus::Tentative,
        Some("cancelled") => EventStatus::Cancelled,
        _ => EventStatus::Confirmed,
    };
    let ev = Event {
        id: None,
        calendar_id: calendar_id.to_string(),
        uid: Some(uid.clone()),
        title: item["summary"].as_str().unwrap_or("(no title)").to_string(),
        description: item["description"].as_str().unwrap_or_default().to_string(),
        location: item["location"].as_str().unwrap_or_default().to_string(),
        start,
        end,
        all_day,
        timezone: tz,
        rrule,
        attendees: item["attendees"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some(Attendee {
                            email: x["email"].as_str()?.to_string(),
                            name: x["displayName"].as_str().map(String::from),
                            status: partstat_from_google(x["responseStatus"].as_str().unwrap_or_default()),
                            optional: x["optional"] == true,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        organizer: item["organizer"]["email"].as_str().map(|e| Address { email: e.to_string(), name: item["organizer"]["displayName"].as_str().map(String::from) }),
        status,
        my_status: None,
    };
    let exdates: Vec<DateTime<Utc>> = item["recurrence"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(Value::as_str)
                .filter(|l| l.starts_with("EXDATE"))
                .flat_map(|l| {
                    let parsed = satchel_ical::parse(&format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:19700101T000000Z\r\n{l}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"));
                    parsed.events.into_iter().flat_map(|e| e.exdates.into_iter().map(|x| x.to_utc())).collect::<Vec<_>>()
                })
                .collect()
        })
        .unwrap_or_default();
    let mut ics = satchel_ical::serialize(&ev, &satchel_ical::SerializeOpts { sequence: item["sequence"].as_i64().unwrap_or(0), exdates: &exdates, ..Default::default() });
    if item["recurringEventId"].is_string()
        && let Some((orig, od, otz)) = gdate(&item["originalStartTime"]) {
            let line = if od {
                format!("RECURRENCE-ID;VALUE=DATE:{}", orig.format("%Y%m%d"))
            } else if let Some(z) = otz.and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
                format!("RECURRENCE-ID;TZID={}:{}", z.name(), orig.with_timezone(&z).format("%Y%m%dT%H%M%S"))
            } else {
                format!("RECURRENCE-ID:{}", orig.format("%Y%m%dT%H%M%SZ"))
            };
            ics = ics.replacen("BEGIN:VEVENT\r\n", &format!("BEGIN:VEVENT\r\n{line}\r\n"), 1);
        }
    Some((uid, ics))
}

/// ICS master component -> Google event body.
pub fn ics_to_google(ics: &str) -> Option<Value> {
    let parsed = satchel_ical::parse(ics);
    let m = parsed.events.iter().find(|e| e.recurrence_id.is_none()).or(parsed.events.first())?;
    let time = |t: &satchel_ical::IcalTime| -> Value {
        match t {
            satchel_ical::IcalTime::Date(d) => json!({ "date": d.format("%Y-%m-%d").to_string() }),
            satchel_ical::IcalTime::Zoned(n, tz) => json!({ "dateTime": n.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": tz }),
            other => json!({ "dateTime": other.to_utc().to_rfc3339(), "timeZone": "UTC" }),
        }
    };
    let end = m.end.clone().unwrap_or_else(|| satchel_ical::IcalTime::Utc(m.end_utc()));
    let mut recurrence = vec![];
    if let Some(r) = &m.rrule {
        recurrence.push(format!("RRULE:{r}"));
    }
    for x in &m.exdates {
        recurrence.push(match x {
            satchel_ical::IcalTime::Date(d) => format!("EXDATE;VALUE=DATE:{}", d.format("%Y%m%d")),
            satchel_ical::IcalTime::Zoned(n, tz) => format!("EXDATE;TZID={tz}:{}", n.format("%Y%m%dT%H%M%S")),
            o => format!("EXDATE:{}", o.to_utc().format("%Y%m%dT%H%M%SZ")),
        });
    }
    let mut body = json!({
        "summary": m.summary,
        "description": m.description,
        "location": m.location,
        "start": time(&m.start),
        "end": time(&end),
        "attendees": m.attendees.iter().map(|a| json!({ "email": a.email, "displayName": a.name, "responseStatus": partstat_to_google(a.status), "optional": a.optional })).collect::<Vec<_>>(),
        "status": match m.status { EventStatus::Tentative => "tentative", EventStatus::Cancelled => "cancelled", EventStatus::Confirmed => "confirmed" },
    });
    if !recurrence.is_empty() {
        body["recurrence"] = json!(recurrence);
    }
    if !m.uid.is_empty() && !m.uid.ends_with("@google.com") {
        body["iCalUID"] = json!(m.uid);
    }
    Some(body)
}

impl GoogleCalendarProvider {
    pub fn new(creds: CredentialHandle, owner: String) -> Result<Self> {
        Ok(Self { api: api_client(creds)?, owner })
    }

    async fn sync_calendar(&self, ctx: &SyncCtx, local: &str, cal: &str, token: Option<String>) -> Result<()> {
        let mut page: Option<String> = None;
        let mut next_sync = None;
        loop {
            let mut url = format!("{GCAL}/calendars/{}/events?maxResults=2500&showDeleted=true", crate::net::enc(cal));
            if let Some(t) = &token {
                url.push_str(&format!("&syncToken={}", crate::net::enc(t)));
            }
            if let Some(p) = &page {
                url.push_str(&format!("&pageToken={}", crate::net::enc(p)));
            }
            let v = match self.api.json(|h, t| h.get(&url).bearer_auth(t), "list events").await {
                Ok(v) => v,
                Err(e) if token.is_some() && e.to_string().contains("HTTP 410") => {
                    ctx.store.set_calendar_token(local, None)?;
                    return Box::pin(self.sync_calendar(ctx, local, cal, None)).await;
                }
                Err(e) => return Err(e),
            };
            for mut item in v["items"].as_array().cloned().unwrap_or_default() {
                let id = item["id"].as_str().unwrap_or_default().to_string();
                let id = id.as_str();
                let cancelled = item["status"] == "cancelled";
                if cancelled && !item["recurringEventId"].is_string() {
                    ctx.delete_event_remote(local, id)?;
                    continue;
                }
                // A deleted occurrence is kept as a cancelled override, which
                // hides it. It may not carry the series' iCalUID; the master has it.
                if cancelled
                    && item["iCalUID"].is_null()
                    && let Some(master) = item["recurringEventId"].as_str().and_then(|m| ctx.store.event_by_remote(local, m).ok().flatten())
                {
                    item["iCalUID"] = json!(master.uid);
                }
                if let Some((_uid, ics)) = google_to_ics(&item, local) {
                    ctx.put_event(local, id, item["etag"].as_str(), &ics)?;
                }
            }
            page = v["nextPageToken"].as_str().map(String::from);
            if let Some(t) = v["nextSyncToken"].as_str() {
                next_sync = Some(t.to_string());
            }
            if page.is_none() {
                break;
            }
        }
        if let Some(t) = next_sync {
            ctx.store.set_calendar_token(local, Some(&t))?;
        }
        Ok(())
    }
}

#[async_trait]
impl CalendarProvider for GoogleCalendarProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let v = self.api.json(|h, t| h.get(format!("{GCAL}/users/me/calendarList?maxResults=250")).bearer_auth(t), "list calendars").await?;
        let cals: Vec<RemoteCalendar> = v["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c["deleted"] != true)
            .map(|c| RemoteCalendar {
                remote_id: c["id"].as_str().unwrap_or_default().to_string(),
                name: c["summaryOverride"].as_str().or(c["summary"].as_str()).unwrap_or("Calendar").to_string(),
                color: c["backgroundColor"].as_str().map(String::from),
                read_only: !matches!(c["accessRole"].as_str(), Some("owner") | Some("writer")),
            })
            .collect();
        let map = ctx.set_calendars(&cals)?;
        for c in &cals {
            let Some(local) = map.get(&c.remote_id) else { continue };
            let token = ctx.store.get_calendar(local)?.and_then(|s| s.sync_token);
            self.sync_calendar(ctx, local, &c.remote_id, token).await?;
        }
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            CalendarOp::Put { calendar_remote_id, event_remote_id, ics, etag, .. } => {
                let body = ics_to_google(ics).context("invalid event")?;
                let cal = crate::net::enc(calendar_remote_id);
                let v = match event_remote_id {
                    None => {
                        let organizer = satchel_ical::parse(ics).events.first().and_then(|e| e.organizer.clone());
                        match organizer.filter(|o| !o.email.eq_ignore_ascii_case(&self.owner)) {
                            // Someone else's meeting: import a private copy.
                            // Inserting would make us the organizer and email
                            // invitations to every attendee.
                            Some(o) => {
                                let mut body = body.clone();
                                body["organizer"] = json!({ "email": o.email, "displayName": o.name });
                                let url = format!("{GCAL}/calendars/{cal}/events/import");
                                self.api.json(|h, t| h.post(&url).bearer_auth(t).json(&body), "import event").await?
                            }
                            None => {
                                let url = format!("{GCAL}/calendars/{cal}/events?sendUpdates=all");
                                self.api.json(|h, t| h.post(&url).bearer_auth(t).json(&body), "create event").await?
                            }
                        }
                    }
                    Some(id) => {
                        let url = format!("{GCAL}/calendars/{cal}/events/{}?sendUpdates=all", crate::net::enc(id));
                        let mut patch = body.clone();
                        if let Some(o) = patch.as_object_mut() {
                            o.remove("iCalUID");
                        }
                        // Only our own response is ours to set; the others'
                        // statuses from the last sync may be out of date.
                        if let Some(list) = patch["attendees"].as_array_mut() {
                            for a in list.iter_mut().filter(|a| !a["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(&self.owner))) {
                                if let Some(o) = a.as_object_mut() {
                                    o.remove("responseStatus");
                                }
                            }
                        }
                        // If-Match: a change made in Google since the last
                        // sync fails with 412 (a conflict) instead of being overwritten.
                        self.api
                            .json(|h, t| {
                                let r = h.patch(&url).bearer_auth(t).json(&patch);
                                match etag {
                                    Some(e) => r.header("If-Match", e.as_str()),
                                    None => r,
                                }
                            }, "update event")
                            .await?
                    }
                };
                out.event_remote_id = v["id"].as_str().map(String::from);
                out.event_etag = v["etag"].as_str().map(String::from);
            }
            CalendarOp::Delete { calendar_remote_id, event_remote_id, notify, .. } => {
                let send = if *notify { "all" } else { "none" };
                let url = format!("{GCAL}/calendars/{}/events/{}?sendUpdates={send}", crate::net::enc(calendar_remote_id), crate::net::enc(event_remote_id));
                match self.api.send(|h, t| h.delete(&url).bearer_auth(t), "delete event").await {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("HTTP 410") || e.to_string().contains("HTTP 404") => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_event_roundtrip() {
        let item = json!({
            "id": "abc", "iCalUID": "abc@google.com", "summary": "Standup", "status": "confirmed",
            "start": { "dateTime": "2026-01-05T09:00:00-05:00", "timeZone": "America/New_York" },
            "end": { "dateTime": "2026-01-05T09:15:00-05:00", "timeZone": "America/New_York" },
            "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;TZID=America/New_York:20260112T090000"],
            "attendees": [{ "email": "me@x.test", "responseStatus": "needsAction", "self": true }],
            "organizer": { "email": "boss@x.test" }
        });
        let (uid, ics) = google_to_ics(&item, "cal").unwrap();
        assert_eq!(uid, "abc@google.com");
        let p = satchel_ical::parse(&ics);
        assert_eq!(p.events[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(p.events[0].exdates.len(), 1);
        let back = ics_to_google(&ics).unwrap();
        assert_eq!(back["start"]["timeZone"], "America/New_York");
        assert_eq!(back["start"]["dateTime"], "2026-01-05T09:00:00");
        assert!(back["recurrence"][0].as_str().unwrap().starts_with("RRULE:"));
        assert!(back.get("iCalUID").is_none());

        let exc = json!({ "id": "abc_20260119T140000Z", "recurringEventId": "abc", "iCalUID": "abc@google.com", "summary": "Standup (moved)",
            "start": { "dateTime": "2026-01-19T11:00:00-05:00" }, "end": { "dateTime": "2026-01-19T11:15:00-05:00" },
            "originalStartTime": { "dateTime": "2026-01-19T09:00:00-05:00", "timeZone": "America/New_York" } });
        let (_, ics2) = google_to_ics(&exc, "cal").unwrap();
        let p2 = satchel_ical::parse(&ics2);
        assert!(p2.events[0].recurrence_id.is_some());
        assert_eq!(p2.events[0].recurrence_id.as_ref().unwrap().to_utc().to_rfc3339(), "2026-01-19T14:00:00+00:00");

        // A deleted occurrence has no start of its own; it becomes a cancelled override.
        let gone = json!({ "id": "abc_20260126T140000Z", "recurringEventId": "abc", "iCalUID": "abc@google.com", "status": "cancelled",
            "originalStartTime": { "dateTime": "2026-01-26T09:00:00-05:00", "timeZone": "America/New_York" } });
        let (_, ics3) = google_to_ics(&gone, "cal").unwrap();
        let p3 = satchel_ical::parse(&ics3);
        assert_eq!(p3.events[0].status, EventStatus::Cancelled);
        assert_eq!(p3.events[0].recurrence_id.as_ref().unwrap().to_utc().to_rfc3339(), "2026-01-26T14:00:00+00:00");
    }
}
