//! Microsoft Graph provider: Outlook / Microsoft 365 mail and calendar.
//!
//! Uses immutable ids so messages keep their id across folder moves. Calendar
//! events are synced as concrete instances (calendarView delta), so recurring
//! series appear as their occurrences.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, MailProvider, RemoteCalendar, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde_json::{Value, json};
use tracing::warn;

use crate::oauth::{ApiClient, OAuthApp, TokenSource};

const ME: &str = "https://graph.microsoft.com/v1.0/me";
const SELECT: &str = "id,conversationId,internetMessageId,subject,from,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,isRead,isDraft,flag,hasAttachments,bodyPreview,parentFolderId";
const WELL_KNOWN: [(&str, MailboxRole); 6] = [
    ("inbox", MailboxRole::Inbox),
    ("sentitems", MailboxRole::Sent),
    ("drafts", MailboxRole::Drafts),
    ("deleteditems", MailboxRole::Trash),
    ("junkemail", MailboxRole::Junk),
    ("archive", MailboxRole::Archive),
];

pub fn api_client(creds: CredentialHandle) -> Result<ApiClient> {
    Ok(ApiClient { http: crate::net::http_client(false)?, tokens: TokenSource::new(OAuthApp::microsoft(), creds)? })
}

fn with_prefs(rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    rb.header("Prefer", "IdType=\"ImmutableId\", outlook.timezone=\"UTC\", odata.maxpagesize=200")
}

fn addr(v: &Value) -> Option<Address> {
    let e = &v["emailAddress"];
    Some(Address { email: e["address"].as_str()?.to_string(), name: e["name"].as_str().filter(|s| !s.is_empty()).map(String::from) })
}

fn addrs(v: &Value) -> Vec<Address> {
    v.as_array().map(|a| a.iter().filter_map(addr).collect()).unwrap_or_default()
}

pub fn record(m: &Value) -> MessageRecord {
    let date = m["receivedDateTime"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or_else(Utc::now);
    MessageRecord {
        remote_id: m["id"].as_str().unwrap_or_default().to_string(),
        remote_thread_id: m["conversationId"].as_str().map(String::from),
        message_id_hdr: m["internetMessageId"].as_str().map(|s| s.trim_matches(['<', '>']).to_string()),
        in_reply_to: None,
        references: vec![],
        subject: m["subject"].as_str().unwrap_or_default().to_string(),
        from: addr(&m["from"]),
        to: addrs(&m["toRecipients"]),
        cc: addrs(&m["ccRecipients"]),
        bcc: addrs(&m["bccRecipients"]),
        reply_to: addrs(&m["replyTo"]),
        date,
        snippet: m["bodyPreview"].as_str().unwrap_or_default().chars().take(200).collect(),
        flags: Flags {
            seen: m["isRead"] == true,
            flagged: m["flag"]["flagStatus"] == "flagged",
            answered: false,
            draft: m["isDraft"] == true,
        },
        attachments: vec![],
        blob_hash: None,
        size: 0,
    }
}

pub struct GraphMailProvider {
    api: ApiClient,
}

impl GraphMailProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: api_client(creds)? })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        self.api.json(|h, t| with_prefs(h.get(url)).bearer_auth(t), what).await
    }

    async fn folders(&self) -> Result<Vec<RemoteMailbox>> {
        let mut roles: HashMap<String, MailboxRole> = HashMap::new();
        for (wk, role) in WELL_KNOWN {
            if let Ok(v) = self.get(&format!("{ME}/mailFolders/{wk}?$select=id"), "well-known folder").await
                && let Some(id) = v["id"].as_str() {
                    roles.insert(id.to_string(), role);
                }
        }
        let mut out = vec![];
        let mut queue: Vec<(String, String)> = vec![(format!("{ME}/mailFolders?$top=250&$select=id,displayName,childFolderCount"), String::new())];
        while let Some((url, prefix)) = queue.pop() {
            let mut next = Some(url);
            while let Some(u) = next {
                let v = self.get(&u, "list folders").await?;
                for f in v["value"].as_array().cloned().unwrap_or_default() {
                    let id = f["id"].as_str().unwrap_or_default().to_string();
                    let name = f["displayName"].as_str().unwrap_or_default().to_string();
                    let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    if f["childFolderCount"].as_u64().unwrap_or(0) > 0 {
                        queue.push((format!("{ME}/mailFolders/{id}/childFolders?$top=250&$select=id,displayName,childFolderCount"), path.clone()));
                    }
                    let role = roles.get(&id).copied().unwrap_or(MailboxRole::Custom);
                    out.push(RemoteMailbox { remote_id: id, name: if role == MailboxRole::Inbox { "Inbox".into() } else { name }, path, role, is_label: false });
                }
                next = v["@odata.nextLink"].as_str().map(String::from);
            }
        }
        // Hide Outlook system folders that aren't mail.
        out.retain(|m| !matches!(m.path.as_str(), "Conversation History" | "Sync Issues" | "Outbox" | "RSS Feeds" | "RSS Subscriptions"));
        Ok(out)
    }

    async fn sync_folder(&self, ctx: &SyncCtx, folder: &RemoteMailbox) -> Result<()> {
        let key = format!("graph:delta:{}", folder.remote_id);
        let mut url = ctx.cursor(&key)?.unwrap_or_else(|| format!("{ME}/mailFolders/{}/messages/delta?$select={SELECT}", folder.remote_id));
        let local_mb = ctx.mailbox_local_id(&folder.remote_id)?;
        let mut n = 0;
        loop {
            let v = match self.get(&url, "message delta").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 410") || e.to_string().contains("syncStateNotFound") => {
                    ctx.store.delete_cursor(ctx.account_id(), &key)?;
                    url = format!("{ME}/mailFolders/{}/messages/delta?$select={SELECT}", folder.remote_id);
                    continue;
                }
                Err(e) => return Err(e),
            };
            for m in v["value"].as_array().cloned().unwrap_or_default() {
                let id = m["id"].as_str().unwrap_or_default();
                if m.get("@removed").is_some() {
                    if let (Some(local), Some(mb)) = (ctx.message_by_remote(id)?, local_mb.as_ref()) {
                        ctx.store.remove_from_mailbox(&local, mb)?;
                    }
                    continue;
                }
                let parent = m["parentFolderId"].as_str().unwrap_or(&folder.remote_id).to_string();
                ctx.put_record(record(&m), &[(parent, None)])?;
                n += 1;
                if n % 200 == 0 {
                    ctx.progress(format!("{}: {n} messages", folder.name));
                }
            }
            if let Some(next) = v["@odata.nextLink"].as_str() {
                url = next.to_string();
                continue;
            }
            if let Some(delta) = v["@odata.deltaLink"].as_str() {
                ctx.set_cursor(&key, delta)?;
            }
            break;
        }
        Ok(())
    }

    async fn mime_upload(&self, url: &str, raw: &[u8], what: &str) -> Result<Value> {
        let body = base64::engine::general_purpose::STANDARD.encode(raw);
        self.api.json(|h, t| with_prefs(h.post(url)).bearer_auth(t).header("Content-Type", "text/plain").body(body.clone()), what).await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        match self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/messages/{id}"))).bearer_auth(t), "delete message").await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("HTTP 404") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl MailProvider for GraphMailProvider {
    async fn test(&self) -> Result<()> {
        self.get(&format!("{ME}?$select=id"), "profile").await?;
        Ok(())
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let folders = self.folders().await?;
        ctx.set_mailboxes(&folders)?;
        let mut order: Vec<&RemoteMailbox> = folders.iter().collect();
        order.sort_by_key(|f| if f.role == MailboxRole::Inbox { 0 } else { 1 });
        for f in order {
            if let Err(e) = self.sync_folder(ctx, f).await {
                warn!(folder = %f.path, "Graph sync: {e:#}");
                return Err(e);
            }
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let r = self.api.send(|h, t| with_prefs(h.get(format!("{ME}/messages/{}/$value", msg.remote_id))).bearer_auth(t), "get MIME").await?;
        Ok(r.bytes().await?.to_vec())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, .. } => {
                let mut patch = serde_json::Map::new();
                if let Some(s) = seen {
                    patch.insert("isRead".into(), json!(s));
                }
                if let Some(f) = flagged {
                    patch.insert("flag".into(), json!({ "flagStatus": if *f { "flagged" } else { "notFlagged" } }));
                }
                if !patch.is_empty() {
                    let body = Value::Object(patch);
                    let urls: Vec<String> = messages.iter().map(|m| format!("{ME}/messages/{}", m.remote_id)).collect();
                    let results: Vec<Result<Value>> = futures::stream::iter(urls.into_iter().map(|url| {
                        let body = body.clone();
                        async move { self.api.json(|h, t| with_prefs(h.patch(&url)).bearer_auth(t).json(&body), "update message").await }
                    }))
                    .buffer_unordered(4)
                    .collect()
                    .await;
                    for r in results {
                        r?;
                    }
                }
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                for m in messages {
                    let v = self
                        .api
                        .json(|h, t| with_prefs(h.post(format!("{ME}/messages/{}/move", m.remote_id))).bearer_auth(t).json(&json!({ "destinationId": to_remote_mailbox })), "move message")
                        .await?;
                    let new_id = v["id"].as_str().unwrap_or(&m.remote_id).to_string();
                    out.rekeyed.push((m.id.clone(), new_id, vec![(to_remote_mailbox.clone(), None)]));
                }
            }
            ResolvedOp::Labels { .. } => anyhow::bail!("Outlook accounts use folders, not labels"),
            ResolvedOp::Expunge { messages } => {
                for m in messages {
                    self.delete(&m.remote_id).await?;
                }
            }
            ResolvedOp::SaveDraft { raw, replaces, .. } => {
                let v = self.mime_upload(&format!("{ME}/messages"), raw, "create draft").await?;
                out.stored_remote_id = v["id"].as_str().map(String::from);
                if let Some(old) = replaces {
                    let _ = self.delete(&old.remote_id).await;
                }
            }
            ResolvedOp::DeleteDraft { message } => self.delete(&message.remote_id).await?,
            ResolvedOp::Send { raw, replaces_draft, .. } => {
                self.mime_upload(&format!("{ME}/sendMail"), raw, "send").await?;
                if let Some(d) = replaces_draft {
                    let _ = self.delete(&d.remote_id).await;
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

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
        uid: e["id"].as_str().map(String::from),
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

impl GraphCalendarProvider {
    pub fn new(creds: CredentialHandle, owner: String) -> Result<Self> {
        Ok(Self { api: api_client(creds)?, owner })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        self.api.json(|h, t| with_prefs(h.get(url)).bearer_auth(t), what).await
    }

    async fn sync_calendar(&self, ctx: &SyncCtx, local: &str, cal: &str) -> Result<()> {
        let stored = ctx.store.get_calendar(local)?.and_then(|c| c.sync_token);
        let start = (Utc::now() - Duration::days(180)).format("%Y-%m-%dT00:00:00Z");
        let end = (Utc::now() + Duration::days(540)).format("%Y-%m-%dT00:00:00Z");
        let mut url = stored.unwrap_or_else(|| format!("{ME}/calendars/{cal}/calendarView/delta?startDateTime={start}&endDateTime={end}"));
        let mut seen: HashSet<String> = HashSet::new();
        loop {
            let v = match self.get(&url, "calendar delta").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 410") => {
                    ctx.store.set_calendar_token(local, None)?;
                    url = format!("{ME}/calendars/{cal}/calendarView/delta?startDateTime={start}&endDateTime={end}");
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
                let full = if e.get("subject").is_none() { self.get(&format!("{ME}/events/{id}"), "get event").await? } else { e };
                if let Some(ics) = graph_to_ics(&full, local, &self.owner) {
                    ctx.put_event(local, &id, full["changeKey"].as_str(), &ics)?;
                    seen.insert(id);
                }
            }
            if let Some(n) = v["@odata.nextLink"].as_str() {
                url = n.to_string();
                continue;
            }
            if let Some(d) = v["@odata.deltaLink"].as_str() {
                ctx.store.set_calendar_token(local, Some(d))?;
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
            CalendarOp::Put { calendar_remote_id, event_remote_id, ics, .. } => {
                let (body, _, organizer) = ics_to_graph(ics).context("invalid event")?;
                let i_organize = organizer.as_ref().is_none_or(|o| o.eq_ignore_ascii_case(&self.owner));
                match event_remote_id {
                    None => {
                        let v = self.api.json(|h, t| with_prefs(h.post(format!("{ME}/calendars/{calendar_remote_id}/events"))).bearer_auth(t).json(&body), "create event").await?;
                        out.event_remote_id = v["id"].as_str().map(String::from);
                        out.event_etag = v["changeKey"].as_str().map(String::from);
                    }
                    Some(id) if i_organize => {
                        let v = self.api.json(|h, t| with_prefs(h.patch(format!("{ME}/events/{id}"))).bearer_auth(t).json(&body), "update event").await?;
                        out.event_remote_id = Some(id.clone());
                        out.event_etag = v["changeKey"].as_str().map(String::from);
                    }
                    Some(id) => {
                        // Attendees can only respond.
                        let mine = satchel_ical::parse(ics).events.iter().flat_map(|e| e.attendees.clone()).find(|a| a.email.eq_ignore_ascii_case(&self.owner)).map(|a| a.status);
                        let action = match mine {
                            Some(PartStat::Accepted) => Some("accept"),
                            Some(PartStat::Declined) => Some("decline"),
                            Some(PartStat::Tentative) => Some("tentativelyAccept"),
                            _ => None,
                        };
                        if let Some(a) = action {
                            self.api.send(|h, t| with_prefs(h.post(format!("{ME}/events/{id}/{a}"))).bearer_auth(t).json(&json!({ "sendResponse": true })), "respond to invite").await?;
                        }
                        out.event_remote_id = Some(id.clone());
                    }
                }
            }
            // Deleting as an attendee only removes our copy; Graph sends a
            // response only through accept/decline, so `notify` needs nothing here.
            CalendarOp::Delete { event_remote_id, .. } => {
                match self.api.send(|h, t| with_prefs(h.delete(format!("{ME}/events/{event_remote_id}"))).bearer_auth(t), "delete event").await {
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
    fn message_record() {
        let m = json!({ "id": "AAMk", "conversationId": "C1", "internetMessageId": "<x@y>", "subject": "Hi",
            "from": { "emailAddress": { "name": "Bob", "address": "bob@x.test" } }, "toRecipients": [{ "emailAddress": { "address": "a@x.test" } }],
            "receivedDateTime": "2026-02-03T04:05:06Z", "isRead": false, "flag": { "flagStatus": "flagged" }, "bodyPreview": "hello" });
        let r = record(&m);
        assert_eq!(r.message_id_hdr.as_deref(), Some("x@y"));
        assert!(!r.flags.seen && r.flags.flagged);
        assert_eq!(r.to[0].email, "a@x.test");
    }

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
    }
}
