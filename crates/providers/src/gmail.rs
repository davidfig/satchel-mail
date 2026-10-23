//! Gmail API mail provider (labels, history-based deltas) and Google Calendar.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, TimeZone, Utc};
use futures::StreamExt;
use satchel_api::*;
use satchel_core::provider::{ApplyOutcome, CalendarOp, CalendarProvider, CredentialHandle, MailProvider, RemoteCalendar, RemoteMailbox, ResolvedOp, SyncCtx};
use satchel_core::store::{MessageRecord, MessageRef};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::oauth::{ApiClient, OAuthApp, TokenSource};

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const GCAL: &str = "https://www.googleapis.com/calendar/v3";
const META_HEADERS: [&str; 10] = ["From", "To", "Cc", "Bcc", "Reply-To", "Subject", "Date", "Message-ID", "In-Reply-To", "References"];
const CONCURRENCY: usize = 8;

pub fn api_client(creds: CredentialHandle) -> Result<ApiClient> {
    Ok(ApiClient { http: crate::net::http_client(false)?, tokens: TokenSource::new(OAuthApp::google(), creds)? })
}

fn system_role(id: &str) -> Option<MailboxRole> {
    Some(match id {
        "INBOX" => MailboxRole::Inbox,
        "SENT" => MailboxRole::Sent,
        "DRAFT" => MailboxRole::Drafts,
        "TRASH" => MailboxRole::Trash,
        "SPAM" => MailboxRole::Junk,
        "STARRED" => MailboxRole::Flagged,
        "IMPORTANT" => MailboxRole::Important,
        _ => return None,
    })
}

fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim_end_matches('=');
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)?)
}

fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn flags_from_labels(labels: &[String]) -> Flags {
    Flags {
        seen: !labels.iter().any(|l| l == "UNREAD"),
        flagged: labels.iter().any(|l| l == "STARRED"),
        answered: false,
        draft: labels.iter().any(|l| l == "DRAFT"),
    }
}

fn label_ids(v: &Value) -> Vec<String> {
    v["labelIds"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

/// Build a record from a `format=metadata` message.
fn record_from_metadata(m: &Value) -> MessageRecord {
    let mut hdr: HashMap<String, String> = HashMap::new();
    for h in m["payload"]["headers"].as_array().cloned().unwrap_or_default() {
        if let (Some(n), Some(v)) = (h["name"].as_str(), h["value"].as_str()) {
            hdr.entry(n.to_ascii_lowercase()).or_insert_with(|| v.to_string());
        }
    }
    // Reuse the MIME parser for address and id parsing by feeding it a header block.
    let mut block = String::new();
    for k in META_HEADERS {
        if let Some(v) = hdr.get(&k.to_ascii_lowercase()) {
            block.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    block.push_str("\r\n");
    let p = satchel_mime::parse(block.as_bytes()).unwrap_or_default();
    let internal = m["internalDate"].as_str().and_then(|s| s.parse::<i64>().ok()).and_then(|ms| Utc.timestamp_millis_opt(ms).single());
    let labels = label_ids(m);
    MessageRecord {
        remote_id: m["id"].as_str().unwrap_or_default().to_string(),
        remote_thread_id: m["threadId"].as_str().map(String::from),
        message_id_hdr: p.message_id,
        in_reply_to: p.in_reply_to,
        references: p.references,
        subject: p.subject,
        from: p.from,
        to: p.to,
        cc: p.cc,
        bcc: p.bcc,
        reply_to: p.reply_to,
        date: internal.or(p.date).unwrap_or_else(Utc::now),
        snippet: html_unescape(m["snippet"].as_str().unwrap_or_default()),
        flags: flags_from_labels(&labels),
        attachments: vec![],
        blob_hash: None,
        size: m["sizeEstimate"].as_u64().unwrap_or(0),
    }
}

/// Mailbox memberships for a set of Gmail labels. Everything outside Trash and
/// Spam is also in the virtual All Mail mailbox.
fn memberships(labels: &[String], known: &HashSet<String>) -> Vec<(String, Option<u32>)> {
    let mut out: Vec<(String, Option<u32>)> = labels.iter().filter(|l| known.contains(*l)).map(|l| (l.clone(), None)).collect();
    if !labels.iter().any(|l| l == "TRASH" || l == "SPAM") {
        out.push(("ALL".into(), None));
    }
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

pub struct GmailProvider {
    api: ApiClient,
}

impl GmailProvider {
    pub fn new(creds: CredentialHandle) -> Result<Self> {
        Ok(Self { api: api_client(creds)? })
    }

    async fn get(&self, url: &str, what: &str) -> Result<Value> {
        self.api.json(|h, t| h.get(url).bearer_auth(t), what).await
    }

    async fn post(&self, url: &str, body: Value, what: &str) -> Result<Value> {
        self.api.json(|h, t| h.post(url).bearer_auth(t).json(&body), what).await
    }

    async fn labels(&self) -> Result<Vec<RemoteMailbox>> {
        let v = self.get(&format!("{GMAIL}/labels"), "list labels").await?;
        let mut out = vec![];
        for l in v["labels"].as_array().cloned().unwrap_or_default() {
            let id = l["id"].as_str().unwrap_or_default().to_string();
            let name = l["name"].as_str().unwrap_or(&id).to_string();
            if id.starts_with("CATEGORY_") || id == "UNREAD" || id == "CHAT" || l["labelListVisibility"] == "labelHide" && l["type"] != "system" {
                continue;
            }
            let (role, display) = match system_role(&id) {
                Some(r) => (r, match r {
                    MailboxRole::Inbox => "Inbox".to_string(),
                    MailboxRole::Sent => "Sent".into(),
                    MailboxRole::Drafts => "Drafts".into(),
                    MailboxRole::Trash => "Trash".into(),
                    MailboxRole::Junk => "Spam".into(),
                    MailboxRole::Flagged => "Starred".into(),
                    MailboxRole::Important => "Important".into(),
                    _ => name.clone(),
                }),
                None if l["type"] == "system" => continue,
                None => (MailboxRole::Custom, name.rsplit('/').next().unwrap_or(&name).to_string()),
            };
            out.push(RemoteMailbox { remote_id: id, name: display, path: name, role, is_label: true });
        }
        Ok(out)
    }

    async fn fetch_metadata(&self, ctx: &SyncCtx, ids: Vec<String>, known_labels: &HashSet<String>) -> Result<()> {
        let q: String = META_HEADERS.iter().map(|h| format!("&metadataHeaders={h}")).collect();
        let total = ids.len();
        let mut done = 0;
        let mut stream = futures::stream::iter(ids.into_iter().map(|id| {
            let url = format!("{GMAIL}/messages/{id}?format=metadata{q}");
            async move { (id, self.get(&url, "get message").await) }
        }))
        .buffer_unordered(CONCURRENCY);
        while let Some((id, r)) = stream.next().await {
            done += 1;
            if done % 100 == 0 {
                ctx.progress(format!("Downloading headers {done}/{total}"));
            }
            match r {
                Ok(m) => {
                    let mbs = memberships(&label_ids(&m), known_labels);
                    ctx.put_record(record_from_metadata(&m), &mbs)?;
                }
                Err(e) if e.to_string().contains("404") => ctx.delete_remote(&id)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    async fn list_ids(&self, q: &str, page: Option<String>, max: usize) -> Result<(Vec<String>, Option<String>)> {
        let mut ids = vec![];
        let mut token = page;
        loop {
            let mut url = format!("{GMAIL}/messages?maxResults=500&includeSpamTrash=true&q={}", crate::net::enc(q));
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = self.get(&url, "list messages").await?;
            ids.extend(v["messages"].as_array().cloned().unwrap_or_default().into_iter().filter_map(|m| m["id"].as_str().map(String::from)));
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() || ids.len() >= max {
                return Ok((ids, token));
            }
        }
    }

    async fn full_sync(&self, ctx: &SyncCtx, labels: &HashSet<String>) -> Result<()> {
        let profile = self.get(&format!("{GMAIL}/profile"), "profile").await?;
        let history = profile["historyId"].as_str().context("no historyId")?.to_string();
        let days = ctx.settings.initial_sync_days.max(7);
        let (ids, _) = self.list_ids(&format!("newer_than:{days}d"), None, usize::MAX).await?;
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let missing: Vec<String> = ids.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        self.fetch_metadata(ctx, missing, labels).await?;
        ctx.set_cursor("gmail:history", &history)?;
        ctx.set_cursor("gmail:backfill", "start")?;
        Ok(())
    }

    /// Older mail, a page at a time across syncs.
    async fn backfill(&self, ctx: &SyncCtx, labels: &HashSet<String>) -> Result<()> {
        let Some(state) = ctx.cursor("gmail:backfill")? else { return Ok(()) };
        if state == "done" {
            return Ok(());
        }
        let days = ctx.settings.initial_sync_days.max(7);
        let page = (state != "start").then_some(state);
        let (ids, next) = self.list_ids(&format!("older_than:{days}d"), page, 1000).await?;
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let missing: Vec<String> = ids.into_iter().filter(|i| !known.contains_key(i)).collect();
        self.fetch_metadata(ctx, missing, labels).await?;
        ctx.set_cursor("gmail:backfill", next.as_deref().unwrap_or("done"))?;
        Ok(())
    }

    async fn delta(&self, ctx: &SyncCtx, start: &str, labels: &HashSet<String>) -> Result<bool> {
        let mut token: Option<String> = None;
        let mut added: HashSet<String> = HashSet::new();
        let mut relabeled: HashMap<String, Vec<String>> = HashMap::new();
        let mut deleted: HashSet<String> = HashSet::new();
        let mut latest = start.to_string();
        loop {
            let mut url = format!("{GMAIL}/history?startHistoryId={start}&maxResults=500");
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = match self.get(&url, "history").await {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 404") => return Ok(false),
                Err(e) => return Err(e),
            };
            for h in v["history"].as_array().cloned().unwrap_or_default() {
                for a in h["messagesAdded"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = a["message"]["id"].as_str() {
                        added.insert(id.to_string());
                        deleted.remove(id);
                    }
                }
                for d in h["messagesDeleted"].as_array().cloned().unwrap_or_default() {
                    if let Some(id) = d["message"]["id"].as_str() {
                        deleted.insert(id.to_string());
                        added.remove(id);
                        relabeled.remove(id);
                    }
                }
                for key in ["labelsAdded", "labelsRemoved"] {
                    for l in h[key].as_array().cloned().unwrap_or_default() {
                        if let Some(id) = l["message"]["id"].as_str() {
                            relabeled.insert(id.to_string(), label_ids(&l["message"]));
                        }
                    }
                }
            }
            if let Some(hid) = v["historyId"].as_str() {
                latest = hid.to_string();
            }
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() {
                break;
            }
        }
        for id in &deleted {
            ctx.delete_remote(id)?;
        }
        let known = ctx.store.remote_ids(ctx.account_id())?;
        let new: Vec<String> = added.iter().filter(|i| !known.contains_key(*i)).cloned().collect();
        self.fetch_metadata(ctx, new, labels).await?;
        for (id, labs) in relabeled {
            if added.contains(&id) && !known.contains_key(&id) {
                continue;
            }
            ctx.set_flags_remote(&id, flags_from_labels(&labs))?;
            ctx.set_memberships_remote(&id, &memberships(&labs, labels))?;
        }
        ctx.set_cursor("gmail:history", &latest)?;
        Ok(true)
    }

    async fn draft_id_for(&self, message_remote_id: &str) -> Result<Option<String>> {
        let mut token: Option<String> = None;
        loop {
            let mut url = format!("{GMAIL}/drafts?maxResults=500");
            if let Some(t) = &token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let v = self.get(&url, "list drafts").await?;
            for d in v["drafts"].as_array().cloned().unwrap_or_default() {
                if d["message"]["id"].as_str() == Some(message_remote_id) {
                    return Ok(d["id"].as_str().map(String::from));
                }
            }
            token = v["nextPageToken"].as_str().map(String::from);
            if token.is_none() {
                return Ok(None);
            }
        }
    }

    async fn modify(&self, ids: Vec<String>, add: Vec<String>, remove: Vec<String>) -> Result<()> {
        for chunk in ids.chunks(1000) {
            self.post(&format!("{GMAIL}/messages/batchModify"), json!({ "ids": chunk, "addLabelIds": add, "removeLabelIds": remove }), "modify labels").await?;
        }
        Ok(())
    }
}

#[async_trait]
impl MailProvider for GmailProvider {
    async fn test(&self) -> Result<()> {
        self.get(&format!("{GMAIL}/profile"), "profile").await?;
        Ok(())
    }

    fn uses_labels(&self) -> bool {
        true
    }

    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let mut mbs = self.labels().await?;
        // Archived messages keep a membership in a virtual All Mail mailbox.
        mbs.push(RemoteMailbox { remote_id: "ALL".into(), name: "All Mail".into(), path: "All Mail".into(), role: MailboxRole::All, is_label: true });
        ctx.set_mailboxes(&mbs)?;
        let labels: HashSet<String> = mbs.iter().map(|m| m.remote_id.clone()).collect();
        match ctx.cursor("gmail:history")? {
            Some(h) => {
                if !self.delta(ctx, &h, &labels).await? {
                    debug!("Gmail history expired; full resync");
                    self.full_sync(ctx, &labels).await?;
                }
            }
            None => self.full_sync(ctx, &labels).await?,
        }
        if let Err(e) = self.backfill(ctx, &labels).await {
            warn!("Gmail backfill: {e:#}");
        }
        Ok(())
    }

    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        let v = self.get(&format!("{GMAIL}/messages/{}?format=raw", msg.remote_id), "get raw message").await?;
        b64url_decode(v["raw"].as_str().context("no raw content")?)
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        let ids = |m: &[MessageRef]| m.iter().map(|x| x.remote_id.clone()).collect::<Vec<_>>();
        match op {
            ResolvedOp::SetFlags { messages, seen, flagged, .. } => {
                let mut add = vec![];
                let mut remove = vec![];
                match seen {
                    Some(true) => remove.push("UNREAD".to_string()),
                    Some(false) => add.push("UNREAD".to_string()),
                    None => {}
                }
                match flagged {
                    Some(true) => add.push("STARRED".to_string()),
                    Some(false) => remove.push("STARRED".to_string()),
                    None => {}
                }
                if !add.is_empty() || !remove.is_empty() {
                    self.modify(ids(messages), add, remove).await?;
                }
            }
            ResolvedOp::Labels { messages, add, remove } => {
                let add: Vec<String> = add.iter().filter(|a| *a != "ALL").cloned().collect();
                let remove: Vec<String> = remove.iter().filter(|a| *a != "ALL").cloned().collect();
                self.modify(ids(messages), add, remove).await?;
            }
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                self.modify(ids(messages), vec![to_remote_mailbox.clone()], vec!["INBOX".into()]).await?;
            }
            ResolvedOp::Expunge { messages } => {
                self.post(&format!("{GMAIL}/messages/batchDelete"), json!({ "ids": ids(messages) }), "delete messages").await?;
            }
            ResolvedOp::SaveDraft { raw, replaces, .. } => {
                let body = json!({ "message": { "raw": b64url(raw) } });
                let existing = match replaces {
                    Some(r) => self.draft_id_for(&r.remote_id).await?,
                    None => None,
                };
                let v = match existing {
                    Some(d) => self.api.json(|h, t| h.put(format!("{GMAIL}/drafts/{d}")).bearer_auth(t).json(&body), "update draft").await?,
                    None => self.post(&format!("{GMAIL}/drafts"), body.clone(), "create draft").await?,
                };
                out.stored_remote_id = v["message"]["id"].as_str().map(String::from);
            }
            ResolvedOp::DeleteDraft { message } => {
                if let Some(d) = self.draft_id_for(&message.remote_id).await? {
                    self.api.send(|h, t| h.delete(format!("{GMAIL}/drafts/{d}")).bearer_auth(t), "delete draft").await?;
                }
            }
            ResolvedOp::Send { raw, replaces_draft, thread_remote_id, .. } => {
                let mut body = json!({ "raw": b64url(raw) });
                if let Some(t) = thread_remote_id {
                    body["threadId"] = json!(t);
                }
                self.post(&format!("{GMAIL}/messages/send"), body, "send").await?;
                if let Some(d) = replaces_draft
                    && let Ok(Some(did)) = self.draft_id_for(&d.remote_id).await {
                        let _ = self.api.send(|h, t| h.delete(format!("{GMAIL}/drafts/{did}")).bearer_auth(t), "delete draft").await;
                    }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Google Calendar
// ---------------------------------------------------------------------------

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
    let (start, all_day, tz) = gdate(&item["start"])?;
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
            for item in v["items"].as_array().cloned().unwrap_or_default() {
                let id = item["id"].as_str().unwrap_or_default();
                let cancelled = item["status"] == "cancelled";
                if cancelled && !item["recurringEventId"].is_string() {
                    ctx.delete_event_remote(local, id)?;
                    continue;
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
        let _ = &self.owner;
        Ok(())
    }

    async fn apply(&self, _ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            CalendarOp::Put { calendar_remote_id, event_remote_id, ics, .. } => {
                let body = ics_to_google(ics).context("invalid event")?;
                let cal = crate::net::enc(calendar_remote_id);
                let v = match event_remote_id {
                    None => {
                        let url = format!("{GCAL}/calendars/{cal}/events?sendUpdates=all");
                        self.api.json(|h, t| h.post(&url).bearer_auth(t).json(&body), "create event").await?
                    }
                    Some(id) => {
                        let url = format!("{GCAL}/calendars/{cal}/events/{}?sendUpdates=all", crate::net::enc(id));
                        let mut patch = body.clone();
                        if let Some(o) = patch.as_object_mut() {
                            o.remove("iCalUID");
                        }
                        self.api.json(|h, t| h.patch(&url).bearer_auth(t).json(&patch), "update event").await?
                    }
                };
                out.event_remote_id = v["id"].as_str().map(String::from);
                out.event_etag = v["etag"].as_str().map(String::from);
            }
            CalendarOp::Delete { calendar_remote_id, event_remote_id, .. } => {
                let url = format!("{GCAL}/calendars/{}/events/{}?sendUpdates=all", crate::net::enc(calendar_remote_id), crate::net::enc(event_remote_id));
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
    fn metadata_to_record() {
        let m = json!({
            "id": "18a", "threadId": "18t", "labelIds": ["INBOX", "UNREAD", "Label_1"], "snippet": "Hi &amp; bye", "internalDate": "1767225600000", "sizeEstimate": 1234,
            "payload": { "headers": [
                { "name": "From", "value": "Bob <bob@x.test>" }, { "name": "To", "value": "a@x.test" }, { "name": "Subject", "value": "Hello" },
                { "name": "Message-ID", "value": "<m1@x>" }, { "name": "In-Reply-To", "value": "<m0@x>" }
            ] }
        });
        let r = record_from_metadata(&m);
        assert_eq!(r.subject, "Hello");
        assert_eq!(r.from.unwrap().email, "bob@x.test");
        assert_eq!(r.message_id_hdr.as_deref(), Some("m1@x"));
        assert_eq!(r.in_reply_to.as_deref(), Some("m0@x"));
        assert!(!r.flags.seen);
        assert_eq!(r.snippet, "Hi & bye");
        assert_eq!(r.remote_thread_id.as_deref(), Some("18t"));
    }

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
    }
}
