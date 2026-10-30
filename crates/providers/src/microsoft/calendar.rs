//! Outlook calendar: events as concrete instances, edits, and RSVPs.

use super::*;

pub struct GraphCalendarProvider {
    api: ApiClient,
    owner: String,
}

fn graph_time(v: &Value) -> Option<DateTime<Utc>> {
    let s = v["dateTime"].as_str()?;
    let s = s.split('.').next().unwrap_or(s);
    let n = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok()?;
    let tz = v["timeZone"].as_str().unwrap_or("UTC");
    match tz.parse::<chrono_tz::Tz>() {
        Ok(z) => z.from_local_datetime(&n).earliest().map(|d| d.with_timezone(&Utc)),
        Err(_) => Some(Utc.from_utc_datetime(&n)),
    }
}

fn graph_partstat(s: &str) -> PartStat {
    match s {
        "accepted" | "organizer" => PartStat::Accepted,
        "declined" => PartStat::Declined,
        "tentativelyAccepted" => PartStat::Tentative,
        _ => PartStat::NeedsAction,
    }
}

/// Graph event instance -> ICS (instances are stored as standalone events).
pub fn graph_to_ics(e: &Value, calendar_id: &str, owner: &str) -> Option<String> {
    let start = graph_time(&e["start"])?;
    let end = graph_time(&e["end"]).unwrap_or(start);
    let mut attendees: Vec<Attendee> = e["attendees"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Attendee {
                        email: x["emailAddress"]["address"].as_str()?.to_string(),
                        name: x["emailAddress"]["name"].as_str().map(String::from),
                        status: graph_partstat(x["status"]["response"].as_str().unwrap_or_default()),
                        optional: x["type"] == "optional",
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let organizer = addr(&e["organizer"]);
    // Graph reports the owner's own response separately.
    if let Some(me) = attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(owner))
        && let Some(r) = e["responseStatus"]["response"].as_str() {
            me.status = graph_partstat(r);
        }
    let ev = Event {
        id: None,
        calendar_id: calendar_id.to_string(),
        // The invitation's UID, so emailed invites match the synced event.
        // `iCalUId` differs per occurrence of a series; `uid` doesn't.
        uid: e["uid"].as_str().or(e["iCalUId"].as_str()).or(e["id"].as_str()).map(String::from),
        title: e["subject"].as_str().unwrap_or("(no title)").to_string(),
        description: e["bodyPreview"].as_str().unwrap_or_default().to_string(),
        location: e["location"]["displayName"].as_str().unwrap_or_default().to_string(),
        start,
        end,
        all_day: e["isAllDay"] == true,
        timezone: None,
        rrule: None,
        attendees,
        organizer,
        status: if e["isCancelled"] == true { EventStatus::Cancelled } else if e["showAs"] == "tentative" { EventStatus::Tentative } else { EventStatus::Confirmed },
        my_status: None,
    };
    Some(satchel_ical::serialize(&ev, &Default::default()))
}

/// RRULE subset -> Graph patternedRecurrence.
pub fn rrule_to_graph(rule: &str, start: DateTime<Utc>, tz: &str) -> Option<Value> {
    let mut parts: HashMap<String, String> = HashMap::new();
    for p in rule.trim_start_matches("RRULE:").split(';') {
        if let Some((k, v)) = p.split_once('=') {
            parts.insert(k.to_ascii_uppercase(), v.to_string());
        }
    }
    let interval: u32 = parts.get("INTERVAL").and_then(|v| v.parse().ok()).unwrap_or(1);
    let local = tz.parse::<chrono_tz::Tz>().map(|z| start.with_timezone(&z).date_naive()).unwrap_or(start.date_naive());
    let days = |v: &str| -> Vec<&'static str> {
        v.split(',')
            .filter_map(|d| match d.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit()) {
                "MO" => Some("monday"),
                "TU" => Some("tuesday"),
                "WE" => Some("wednesday"),
                "TH" => Some("thursday"),
                "FR" => Some("friday"),
                "SA" => Some("saturday"),
                "SU" => Some("sunday"),
                _ => None,
            })
            .collect()
    };
    let pattern = match parts.get("FREQ")?.as_str() {
        "DAILY" => json!({ "type": "daily", "interval": interval }),
        "WEEKLY" => {
            let d = parts.get("BYDAY").map(|v| days(v)).unwrap_or_else(|| vec![["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"][local.weekday().num_days_from_monday() as usize]]);
            json!({ "type": "weekly", "interval": interval, "daysOfWeek": d, "firstDayOfWeek": "monday" })
        }
        "MONTHLY" => json!({ "type": "absoluteMonthly", "interval": interval, "dayOfMonth": local.day() }),
        "YEARLY" => json!({ "type": "absoluteYearly", "interval": interval, "dayOfMonth": local.day(), "month": local.month() }),
        _ => return None,
    };
    let start_date = local.format("%Y-%m-%d").to_string();
    let range = if let Some(c) = parts.get("COUNT").and_then(|v| v.parse::<u32>().ok()) {
        json!({ "type": "numbered", "startDate": start_date, "numberOfOccurrences": c })
    } else if let Some(u) = parts.get("UNTIL") {
        let d = &u[..8.min(u.len())];
        let end = format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8]);
        json!({ "type": "endDate", "startDate": start_date, "endDate": end })
    } else {
        json!({ "type": "noEnd", "startDate": start_date })
    };
    Some(json!({ "pattern": pattern, "range": range }))
}

fn ics_to_graph(ics: &str) -> Option<(Value, Option<PartStat>, Option<String>)> {
    let p = satchel_ical::parse(ics);
    let m = p.events.iter().find(|e| e.recurrence_id.is_none()).or(p.events.first())?;
    let tz = m.start.tzid().unwrap_or("UTC").to_string();
    let fmt = |d: DateTime<Utc>| -> Value {
        match tz.parse::<chrono_tz::Tz>() {
            Ok(z) if tz != "UTC" => json!({ "dateTime": d.with_timezone(&z).format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": tz }),
            _ => json!({ "dateTime": d.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC" }),
        }
    };
    let mut body = json!({
        "subject": m.summary,
        "body": { "contentType": "text", "content": m.description },
        "location": { "displayName": m.location },
        "start": fmt(m.start_utc()),
        "end": fmt(m.end_utc()),
        "isAllDay": m.start.is_date(),
        "attendees": m.attendees.iter().map(|a| json!({ "emailAddress": { "address": a.email, "name": a.name }, "type": if a.optional { "optional" } else { "required" } })).collect::<Vec<_>>(),
    });
    if let Some(r) = &m.rrule
        && let Some(rec) = rrule_to_graph(r, m.start_utc(), &tz) {
            body["recurrence"] = rec;
        }
    Some((body, None, m.organizer.as_ref().map(|o| o.email.clone())))
}

/// The Graph response action for our status in `ics`, if it differs from
/// what Outlook already has (`server.responseStatus`).
fn response_action(server: &Value, ics: &str, owner: &str) -> Option<&'static str> {
    let mine = satchel_ical::parse(ics).events.iter().flat_map(|e| e.attendees.clone()).find(|a| a.email.eq_ignore_ascii_case(owner)).map(|a| a.status)?;
    let action = match mine {
        PartStat::Accepted => "accept",
        PartStat::Declined => "decline",
        PartStat::Tentative => "tentativelyAccept",
        PartStat::NeedsAction => return None,
    };
    (graph_partstat(server["responseStatus"]["response"].as_str().unwrap_or_default()) != mine).then_some(action)
}

/// Drop from an event PATCH what the user didn't change. Satchel only knows
/// an event's `bodyPreview` (plain text, ~255 characters) and its location's
/// display name, so sending them back unchanged would replace the real body
/// (Teams link, agenda) and the room. Unchanged attendees stay out too.
fn trim_unchanged(patch: &mut Value, server: &Value) {
    let Some(o) = patch.as_object_mut() else { return };
    let same_text = |a: &Value, b: &Value| a.as_str().unwrap_or_default().trim() == b.as_str().unwrap_or_default().trim();
    if same_text(&o["body"]["content"], &server["bodyPreview"]) {
        o.remove("body");
    }
    if same_text(&o["location"]["displayName"], &server["location"]["displayName"]) {
        o.remove("location");
    }
    let people = |v: &Value| -> Vec<(String, String)> {
        let mut p: Vec<(String, String)> = v
            .as_array()
            .map(|a| a.iter().map(|x| (x["emailAddress"]["address"].as_str().unwrap_or_default().to_lowercase(), x["type"].as_str().unwrap_or("required").to_string())).collect())
            .unwrap_or_default();
        p.sort();
        p
    };
    if people(&o["attendees"]) == people(&server["attendees"]) {
        o.remove("attendees");
    }
}

impl GraphCalendarProvider {
    pub fn new(creds: CredentialHandle, owner: String) -> Result<Self> {
        Ok(Self { api: api_client(creds)?, owner })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        graph_link(url)?;
        self.api.json(|h, t| with_prefs(h.get(url)).bearer_auth(t), what).await
    }

    /// Someone else's meeting: we can only respond, and only when our answer
    /// changed (otherwise every local edit would email the organizer again).
    /// An answer to one occurrence of a series is an answer to the series,
    /// as with an emailed invitation.
    async fn respond(&self, server: &Value, id: &str, ics: &str) -> Result<()> {
        let Some(action) = response_action(server, ics, &self.owner) else { return Ok(()) };
        let target = match (server["type"].as_str(), server["seriesMasterId"].as_str()) {
            (Some("occurrence"), Some(master)) => master,
            _ => id,
        };
        self.api.send(|h, t| with_prefs(h.post(format!("{ME}/events/{}/{action}", crate::net::enc(target)))).bearer_auth(t).json(&json!({ "sendResponse": true })), "respond to invite").await?;
        Ok(())
    }

    async fn sync_calendar(&self, ctx: &SyncCtx, local: &str, cal: &str) -> Result<()> {
        let stored = ctx.store.get_calendar(local)?.and_then(|c| c.sync_token);
        let start = (Utc::now() - Duration::days(180)).format("%Y-%m-%dT00:00:00Z");
        let end = (Utc::now() + Duration::days(540)).format("%Y-%m-%dT00:00:00Z");
        let fresh = format!("{ME}/calendars/{}/calendarView/delta?startDateTime={start}&endDateTime={end}", crate::net::enc(cal));
        // A saved token that isn't a Graph URL is dropped for a fresh sync.
        let mut url = stored.filter(|u| is_graph(u)).unwrap_or_else(|| fresh.clone());
        let mut seen: HashSet<String> = HashSet::new();
        let mut reset = false;
        loop {
            let v = match self.get(&url, "calendar delta").await {
                Ok(v) => v,
                // Expired token: start over, but only once a pass.
                Err(e) if !reset && e.to_string().contains("HTTP 410") => {
                    ctx.store.set_calendar_token(local, None)?;
                    url = fresh.clone();
                    reset = true;
                    continue;
                }
                Err(e) => return Err(e),
            };
            for e in v["value"].as_array().cloned().unwrap_or_default() {
                let id = e["id"].as_str().unwrap_or_default().to_string();
                if e.get("@removed").is_some() || e["isCancelled"] == true {
                    ctx.delete_event_remote(local, &id)?;
                    continue;
                }
                // Delta pages carry only ids for some changes; fetch the full instance.
                let full = if e.get("subject").is_none() { self.get(&format!("{ME}/events/{}", crate::net::enc(&id)), "get event").await? } else { e };
                if let Some(ics) = graph_to_ics(&full, local, &self.owner) {
                    ctx.put_event(local, &id, full["changeKey"].as_str(), &ics)?;
                    seen.insert(id);
                }
            }
            if let Some(n) = v["@odata.nextLink"].as_str() {
                url = graph_link(n)?;
                continue;
            }
            if let Some(d) = v["@odata.deltaLink"].as_str() {
                ctx.store.set_calendar_token(local, Some(&graph_link(d)?))?;
            }
            break;
        }
        Ok(())
    }
}

#[async_trait]
impl CalendarProvider for GraphCalendarProvider {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let v = self.get(&format!("{ME}/calendars?$top=100&$select=id,name,hexColor,canEdit"), "list calendars").await?;
        let cals: Vec<RemoteCalendar> = v["value"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|c| RemoteCalendar {
                remote_id: c["id"].as_str().unwrap_or_default().to_string(),
                name: c["name"].as_str().unwrap_or("Calendar").to_string(),
                color: c["hexColor"].as_str().filter(|s| s.starts_with('#')).map(String::from),
                read_only: c["canEdit"] == false,
            })
            .collect();
        let map = ctx.set_calendars(&cals)?;
        for c in &cals {
            if let Some(local) = map.get(&c.remote_id) {
                self.sync_calendar(ctx, local, &c.remote_id).await?;
            }
        }
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            CalendarOp::Put { calendar_remote_id, event_remote_id, ics, etag, .. } => {
                let (body, _, organizer) = ics_to_graph(ics).context("invalid event")?;
                let i_organize = organizer.as_ref().is_none_or(|o| o.eq_ignore_ascii_case(&self.owner));
                match event_remote_id {
                    // Creating an event makes us its organizer and invites
                    // every attendee; never do that with someone else's meeting.
                    None if !i_organize => {
                        return Err(ApiError::new(ErrorCode::InvalidInput, "Outlook can't add someone else's meeting without inviting its attendees").into());
                    }
                    None => {
                        let v = self.api.json(|h, t| with_prefs(h.post(format!("{ME}/calendars/{}/events", crate::net::enc(calendar_remote_id)))).bearer_auth(t).json(&body), "create event").await?;
                        out.event_remote_id = v["id"].as_str().map(String::from);
                        out.event_etag = v["changeKey"].as_str().map(String::from);
                    }
                    Some(id) => {
                        let server = self
                            .get(&format!("{ME}/events/{}?$select=bodyPreview,location,attendees,changeKey,isOrganizer,responseStatus,seriesMasterId,type", crate::net::enc(id)), "get event")
                            .await?;
                        // Outlook knows whose meeting it is; the organizer's address
                        // may be an alias of the account's.
                        if !server["isOrganizer"].as_bool().unwrap_or(i_organize) {
                            self.respond(&server, id, ics).await?;
                            out.event_remote_id = Some(id.clone());
                            return Ok(out);
                        }
                        // Changed in Outlook since we last synced: don't overwrite it.
                        if let Some(e) = etag
                            && server["changeKey"].as_str().is_some_and(|k| k != e)
                        {
                            return Err(ApiError::new(ErrorCode::Conflict, "the event changed in Outlook since it was last synced; edit it again after the next sync").into());
                        }
                        let mut body = body;
                        trim_unchanged(&mut body, &server);
                        let v = self.api.json(|h, t| with_prefs(h.patch(format!("{ME}/events/{}", crate::net::enc(id)))).bearer_auth(t).json(&body), "update event").await?;
                        out.event_remote_id = Some(id.clone());
                        out.event_etag = v["changeKey"].as_str().map(String::from);
                    }
                }
            }
            // Deleting as an attendee only removes our copy; Graph sends a
            // response only through accept/decline, so `notify` needs nothing here.
            CalendarOp::Delete { event_remote_id, .. } => {
                match self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/events/{}", crate::net::enc(event_remote_id)))).bearer_auth(t), "delete event").await {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("HTTP 404") => {}
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
    fn recurrence_conversion() {
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 14, 0, 0).unwrap();
        let r = rrule_to_graph("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10", start, "America/New_York").unwrap();
        assert_eq!(r["pattern"]["type"], "weekly");
        assert_eq!(r["pattern"]["daysOfWeek"], json!(["monday", "wednesday"]));
        assert_eq!(r["range"]["numberOfOccurrences"], 10);
        let m = rrule_to_graph("FREQ=MONTHLY;UNTIL=20261231T000000Z", start, "UTC").unwrap();
        assert_eq!(m["pattern"]["dayOfMonth"], 5);
        assert_eq!(m["range"]["endDate"], "2026-12-31");
    }

    #[test]
    fn event_to_ics() {
        let e = json!({ "id": "E1", "subject": "Review", "start": { "dateTime": "2026-03-01T15:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-03-01T16:00:00.0000000", "timeZone": "UTC" }, "isAllDay": false,
            "attendees": [{ "emailAddress": { "address": "me@x.test" }, "status": { "response": "none" } }], "responseStatus": { "response": "tentativelyAccepted" } });
        let ics = graph_to_ics(&e, "cal", "me@x.test").unwrap();
        let p = satchel_ical::parse(&ics);
        assert_eq!(p.events[0].attendees[0].status, PartStat::Tentative);
        assert_eq!(p.events[0].start_utc().to_rfc3339(), "2026-03-01T15:00:00+00:00");
        // Without the invitation's UID, Graph's own id stands in.
        assert_eq!(p.events[0].uid, "E1");
        // With it, emailed invitations match the synced event.
        let mut e = e;
        e["iCalUId"] = json!("040000008200E00074C5B7101A82E008-occurrence");
        e["uid"] = json!("040000008200E00074C5B7101A82E008");
        assert_eq!(satchel_ical::parse(&graph_to_ics(&e, "cal", "me@x.test").unwrap()).events[0].uid, "040000008200E00074C5B7101A82E008");
    }

    #[test]
    fn responds_only_when_the_answer_changed() {
        let e = json!({ "id": "E1", "subject": "Sync", "start": { "dateTime": "2026-03-01T15:00:00", "timeZone": "UTC" }, "end": { "dateTime": "2026-03-01T16:00:00", "timeZone": "UTC" },
            "attendees": [{ "emailAddress": { "address": "me@x.test" }, "status": { "response": "none" } }], "responseStatus": { "response": "accepted" } });
        let ics = graph_to_ics(&e, "cal", "me@x.test").unwrap();
        assert_eq!(response_action(&e, &ics, "me@x.test"), None, "already accepted in Outlook");
        let declined = ics.replace("PARTSTAT=ACCEPTED", "PARTSTAT=DECLINED");
        assert_eq!(response_action(&e, &declined, "me@x.test"), Some("decline"));
    }

    #[test]
    fn edits_leave_the_body_and_room_alone() {
        let server = json!({ "bodyPreview": "Join on Teams: https://teams…", "location": { "displayName": "Room 4" },
            "attendees": [{ "emailAddress": { "address": "Bob@x.test" }, "type": "required" }] });
        let mut patch = json!({ "subject": "Review (moved)", "body": { "contentType": "text", "content": "Join on Teams: https://teams…" },
            "location": { "displayName": "Room 4" }, "attendees": [{ "emailAddress": { "address": "bob@x.test" }, "type": "required" }] });
        trim_unchanged(&mut patch, &server);
        assert_eq!(patch, json!({ "subject": "Review (moved)" }));
        let mut changed = json!({ "body": { "content": "New agenda" }, "location": { "displayName": "Room 5" }, "attendees": [] });
        trim_unchanged(&mut changed, &server);
        assert!(changed.get("body").is_some() && changed.get("location").is_some() && changed.get("attendees").is_some());
    }
}
