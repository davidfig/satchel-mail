//! Proposal files in the AI folder (`proposals/inbox/`).
//!
//! Agents propose changes by writing a Markdown file with a front matter
//! header (or a JSON file in the API's shape). The engine ingests each file
//! as a proposal and moves it to `proposals/processed/`, where its `status:`
//! is kept up to date as the user decides.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
use satchel_api::*;

use crate::mirror::root;
use crate::store::Store;

/// A proposal read from a file, before it is attributed to an agent.
#[derive(Debug, serde::Deserialize)]
pub struct DroppedProposal {
    /// Optional: files without a token come from the built-in AI folder agent.
    #[serde(default)]
    pub agent_token: Option<String>,
    /// A self-declared name ("Claude"), shown in Review but not trusted.
    #[serde(default)]
    pub agent: Option<String>,
    pub action: ProposalAction,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub source_thread_ids: Vec<String>,
}

pub fn pending(settings: &MirrorSettings) -> Vec<PathBuf> {
    let dir = root(settings).join("proposals/inbox");
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json" || x == "md"))
        .collect();
    v.sort();
    v
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "md")
}

/// Read one inbox file. Markdown is resolved against the store (thread ids to
/// message ids, mailbox and calendar names to ids).
pub fn read(store: &Store, path: &Path) -> Result<DroppedProposal> {
    let text = std::fs::read_to_string(path)?;
    if is_markdown(path) {
        from_markdown(store, &text)
    } else {
        Ok(serde_json::from_str(&text)?)
    }
}

/// The result of ingesting a file, written back for the agent to read.
pub enum Outcome<'a> {
    Created(&'a Proposal),
    Error(String),
}

/// Move an ingested file to `proposals/processed/` with its outcome. The
/// token is stripped; the name is kept so the agent can find it.
pub fn finish(settings: &MirrorSettings, path: &Path, outcome: Outcome) {
    let done = root(settings).join("proposals/processed");
    let _ = std::fs::create_dir_all(&done);
    let target = free_name(&done, path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    if let Ok(text) = std::fs::read_to_string(path) {
        let out = if is_markdown(path) {
            let mut fields = vec![("agent_token", None)];
            match &outcome {
                Outcome::Created(p) => fields.extend(status_fields(p, false)),
                Outcome::Error(e) => fields.extend([("status", Some("error".to_string())), ("error", Some(quote(e)))]),
            }
            set_fields(&text, &fields)
        } else {
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text }));
            if let Some(o) = v.as_object_mut() {
                o.remove("agent_token");
                let result = match &outcome {
                    Outcome::Created(p) => serde_json::json!({ "ok": p }),
                    Outcome::Error(e) => serde_json::json!({ "error": e }),
                };
                o.insert("result".into(), result);
                if let Outcome::Created(p) = &outcome {
                    o.insert("status".into(), status_word(p, false).into());
                }
            }
            serde_json::to_string_pretty(&v).unwrap_or_default()
        };
        let _ = std::fs::write(&target, out);
    }
    let _ = std::fs::remove_file(path);
}

/// Rewrite the status of the processed file for `p`, if there is one.
/// Decisions are human-paced, so scanning the folder is fine.
pub fn update_status(settings: &MirrorSettings, p: &Proposal, edited: bool) {
    let dir = root(settings).join("proposals/processed");
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for path in rd.filter_map(|e| e.ok().map(|e| e.path())) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if !text.contains(&p.id) {
            continue;
        }
        let out = if is_markdown(&path) {
            let Ok((fm, _)) = parse(&text) else { continue };
            if fm.str("proposal") != Some(p.id.as_str()) {
                continue;
            }
            set_fields(&text, &status_fields(p, edited))
        } else {
            let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            if v["result"]["ok"]["id"].as_str() != Some(p.id.as_str()) {
                continue;
            }
            v["status"] = status_word(p, edited).into();
            v["note"] = p.review_note.clone().into();
            serde_json::to_string_pretty(&v).unwrap_or_default()
        };
        let _ = std::fs::write(&path, out);
        return;
    }
}

fn status_word(p: &Proposal, edited: bool) -> &'static str {
    match p.status {
        ProposalStatus::Pending => "pending",
        ProposalStatus::Approved | ProposalStatus::Applied if edited => "edited",
        ProposalStatus::Approved | ProposalStatus::Applied => "approved",
        ProposalStatus::Rejected => "rejected",
        ProposalStatus::Stale => "stale",
        ProposalStatus::Failed => "failed",
    }
}

fn status_fields(p: &Proposal, edited: bool) -> Vec<(&'static str, Option<String>)> {
    vec![
        ("status", Some(status_word(p, edited).to_string())),
        ("proposal", Some(p.id.clone())),
        ("summary", Some(quote(&p.summary))),
        ("warnings", (!p.warnings.is_empty()).then(|| format!("[{}]", p.warnings.iter().map(|w| quote(w)).collect::<Vec<_>>().join(", ")))),
        ("note", p.review_note.as_deref().map(quote)),
        ("error", None),
    ]
}

fn free_name(dir: &Path, name: String) -> PathBuf {
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), format!(".{e}"))).unwrap_or((name.clone(), String::new()));
    (2..).map(|n| dir.join(format!("{stem}-{n}{ext}"))).find(|p| !p.exists()).unwrap()
}

// ---------------------------------------------------------------------------
// Front matter: `key: value` lines between `---` fences. Values are plain or
// quoted strings, inline lists (`[a, b]`) or `- item` lines under a key.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
}

#[derive(Debug, Default)]
pub struct FrontMatter(pub Vec<(String, FmValue)>);

impl FrontMatter {
    fn get(&self, key: &str) -> Option<&FmValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            FmValue::Str(s) if !s.is_empty() => Some(s),
            FmValue::List(v) if v.len() == 1 => Some(&v[0]),
            _ => None,
        }
    }

    /// A list, accepting a single plain value as a one-item list.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(FmValue::List(v)) => v.clone(),
            Some(FmValue::Str(s)) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    }

    fn bool(&self, key: &str) -> Result<Option<bool>> {
        match self.str(key) {
            None => Ok(None),
            Some("true" | "yes") => Ok(Some(true)),
            Some("false" | "no") => Ok(Some(false)),
            Some(v) => bail!("{key}: expected true or false, got {v:?}"),
        }
    }
}

/// Split a Markdown file into its front matter and body.
pub fn parse(text: &str) -> Result<(FrontMatter, String)> {
    let (header, body) = split(text).ok_or_else(|| anyhow!("missing front matter: start the file with a `---` line"))?;
    let mut fm = FrontMatter::default();
    for (n, line) in header.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ").or((trimmed == "-").then_some("")) {
            match fm.0.last_mut() {
                Some((_, FmValue::List(v))) => v.push(scalar(item)),
                Some((_, v @ FmValue::Str(_))) if *v == FmValue::Str(String::new()) => *v = FmValue::List(vec![scalar(item)]),
                _ => bail!("front matter line {}: list item without a key", n + 1),
            }
            continue;
        }
        let (k, v) = trimmed.split_once(':').ok_or_else(|| anyhow!("front matter line {}: expected `key: value`", n + 1))?;
        let v = strip_comment(v.trim());
        let value = if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            FmValue::List(split_list(inner).iter().map(|s| scalar(s)).filter(|s| !s.is_empty()).collect())
        } else {
            FmValue::Str(scalar(v))
        };
        fm.0.push((k.trim().to_lowercase(), value));
    }
    Ok((fm, body.to_string()))
}

fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start_matches('\u{feff}').strip_prefix("---")?;
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// Drop a trailing ` # comment` outside quotes.
fn strip_comment(v: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in v.char_indices() {
        match quote {
            Some(q) if c == q && prev != '\\' => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev.is_whitespace() => return v[..i].trim_end(),
            None => {}
        }
        prev = c;
    }
    v
}

/// Split an inline list on commas outside quotes.
fn split_list(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c == ',' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn scalar(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new();
        let mut esc = false;
        for c in v[1..v.len() - 1].chars() {
            match (esc, c) {
                (true, 'n') => out.push('\n'),
                (true, c) => out.push(c),
                (false, '\\') => {
                    esc = true;
                    continue;
                }
                (false, c) => out.push(c),
            }
            esc = false;
        }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// A double-quoted value safe to put on one front matter line.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// Set (Some) or remove (None) top-level keys, keeping every other line as written.
pub fn set_fields(text: &str, fields: &[(&str, Option<String>)]) -> String {
    let Some((header, body)) = split(text) else { return text.to_string() };
    let mut lines: Vec<String> = vec![];
    let mut skipping = false;
    for line in header.lines() {
        let is_item = line.trim_start().starts_with('-');
        if skipping && (is_item || line.starts_with(' ')) {
            continue;
        }
        skipping = false;
        let key = line.split_once(':').map(|(k, _)| k.trim().to_lowercase());
        if !line.starts_with(' ') && key.as_deref().is_some_and(|k| fields.iter().any(|(f, _)| *f == k)) {
            skipping = true;
            continue;
        }
        lines.push(line.to_string());
    }
    for (k, v) in fields {
        if let Some(v) = v {
            lines.push(format!("{k}: {v}"));
        }
    }
    format!("---\n{}\n---\n{body}", lines.join("\n"))
}

// ---------------------------------------------------------------------------
// Markdown → proposal
// ---------------------------------------------------------------------------

fn from_markdown(store: &Store, text: &str) -> Result<DroppedProposal> {
    let (fm, body) = parse(text)?;
    let body = body.trim().to_string();
    let action_name = fm.str("action").ok_or_else(|| anyhow!("missing `action:`"))?.to_lowercase().replace('-', "_");
    let threads: Vec<String> = fm.list("thread").into_iter().chain(fm.list("threads")).collect();
    let messages = || -> Result<Vec<String>> {
        let mut ids: Vec<String> = fm.list("message").into_iter().chain(fm.list("messages")).collect();
        for t in &threads {
            let found = store.thread_message_ids(t)?;
            if found.is_empty() {
                bail!("unknown thread {t}");
            }
            ids.extend(found);
        }
        if ids.is_empty() {
            bail!("give `threads:` or `messages:`");
        }
        ids.dedup();
        Ok(ids)
    };
    let action = match action_name.as_str() {
        "reply" => ProposalAction::Draft { draft: reply(store, &fm, &threads, body)? },
        "draft" | "new" | "compose" => {
            let account_id = account(store, fm.str("account"))?;
            ProposalAction::Draft {
                draft: Draft {
                    account_id,
                    to: addresses(&fm, "to")?,
                    cc: addresses(&fm, "cc")?,
                    bcc: addresses(&fm, "bcc")?,
                    subject: fm.str("subject").unwrap_or_default().to_string(),
                    body_text: body,
                    ..Default::default()
                },
            }
        }
        "archive" => ProposalAction::Archive { message_ids: messages()? },
        "move" => {
            let ids = messages()?;
            let to = fm.str("mailbox").or(fm.str("to")).ok_or_else(|| anyhow!("missing `mailbox:`"))?;
            ProposalAction::Move { mailbox_id: mailbox(store, &ids[0], to)?, message_ids: ids }
        }
        "labels" | "label" => {
            let ids = messages()?;
            let resolve = |key: &str| fm.list(key).iter().map(|l| mailbox(store, &ids[0], l)).collect::<Result<Vec<_>>>();
            ProposalAction::Labels { add: resolve("add")?, remove: resolve("remove")?, message_ids: ids }
        }
        "mark_read" => ProposalAction::MarkRead { message_ids: messages()?, seen: true },
        "mark_unread" => ProposalAction::MarkRead { message_ids: messages()?, seen: false },
        "flag" => ProposalAction::Flag { message_ids: messages()?, flagged: true },
        "unflag" => ProposalAction::Flag { message_ids: messages()?, flagged: false },
        "create_event" | "event" => {
            let calendar_id = calendar(store, fm.str("calendar"), fm.str("account"))?;
            let mut e = Event {
                id: None,
                calendar_id,
                uid: None,
                title: String::new(),
                description: String::new(),
                location: String::new(),
                start: Utc::now(),
                end: Utc::now(),
                all_day: false,
                timezone: None,
                rrule: None,
                attendees: vec![],
                organizer: None,
                status: EventStatus::Confirmed,
                my_status: None,
            };
            if fm.str("start").is_none() {
                bail!("missing `start:`");
            }
            apply_event_fields(&mut e, &fm, body)?;
            if e.title.is_empty() {
                bail!("missing `title:`");
            }
            ProposalAction::CreateEvent { event: e }
        }
        "update_event" => {
            let id = fm.str("event").ok_or_else(|| anyhow!("missing `event:` (an event_id from AGENDA.md)"))?;
            let mut e = crate::calendar::get_event(store, id)?.ok_or_else(|| anyhow!("unknown event {id}"))?;
            apply_event_fields(&mut e, &fm, body)?;
            ProposalAction::UpdateEvent { event: e }
        }
        "cancel_event" => ProposalAction::CancelEvent { event_id: fm.str("event").ok_or_else(|| anyhow!("missing `event:`"))?.to_string() },
        "respond_to_invite" | "respond" => {
            let response = match fm.str("response").unwrap_or_default().to_lowercase().as_str() {
                "accept" | "accepted" | "yes" => PartStat::Accepted,
                "decline" | "declined" | "no" => PartStat::Declined,
                "tentative" | "maybe" => PartStat::Tentative,
                other => bail!("response: expected accept, decline or tentative, got {other:?}"),
            };
            ProposalAction::RespondToInvite { event_id: fm.str("event").ok_or_else(|| anyhow!("missing `event:`"))?.to_string(), response }
        }
        other => bail!(
            "unknown action {other:?}; use reply, draft, archive, move, labels, mark_read, mark_unread, flag, unflag, create_event, update_event, cancel_event or respond_to_invite"
        ),
    };
    let why = fm.str("why").or(fm.str("rationale")).unwrap_or_default().to_string();
    Ok(DroppedProposal {
        agent_token: fm.str("agent_token").map(str::to_string),
        agent: fm.str("agent").map(str::to_string),
        action,
        rationale: why,
        source_thread_ids: threads,
    })
}

/// A reply to the last message of `thread` (or to `message:`), addressed to
/// its sender unless `to:` says otherwise.
fn reply(store: &Store, fm: &FrontMatter, threads: &[String], body: String) -> Result<Draft> {
    let message_id = match fm.str("message").or(fm.str("in_reply_to")) {
        Some(m) => m.to_string(),
        None => {
            let t = threads.first().ok_or_else(|| anyhow!("missing `thread:` (a thread_id from thread.md or meta.json)"))?;
            store.thread_message_ids(t)?.pop().ok_or_else(|| anyhow!("unknown thread {t}"))?
        }
    };
    let m = store.message_row(&message_id)?.ok_or_else(|| anyhow!("unknown message {message_id}"))?;
    let own = store.get_account(&m.summary.account_id)?.map(|a| a.email.to_lowercase()).unwrap_or_default();
    let mut to = addresses(fm, "to")?;
    if to.is_empty() {
        let sender: Vec<Address> = if m.reply_to.is_empty() { m.summary.from.iter().cloned().collect() } else { m.reply_to.clone() };
        // Replying to our own message continues to its recipients.
        to = if sender.iter().all(|a| a.email.to_lowercase() == own) { m.summary.to.clone() } else { sender };
    }
    Ok(Draft {
        account_id: m.summary.account_id.clone(),
        to,
        cc: addresses(fm, "cc")?,
        bcc: addresses(fm, "bcc")?,
        subject: fm.str("subject").map(str::to_string).unwrap_or_else(|| satchel_mime::reply_subject(&m.summary.subject)),
        body_text: body,
        in_reply_to: Some(m.summary.id),
        ..Default::default()
    })
}

/// Parse `Name <email>` or a bare address.
fn address(s: &str) -> Result<Address> {
    let s = s.trim();
    let (name, email) = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => (Some(s[..a].trim().trim_matches('"').to_string()).filter(|n| !n.is_empty()), s[a + 1..b].trim()),
        _ => (None, s),
    };
    if !email.contains('@') || email.contains(char::is_whitespace) {
        bail!("not an email address: {s:?}");
    }
    Ok(Address { name, email: email.to_string() })
}

fn addresses(fm: &FrontMatter, key: &str) -> Result<Vec<Address>> {
    fm.list(key).iter().map(|s| address(s).with_context(|| key.to_string())).collect()
}

/// An account by id or email; may be omitted when there is only one.
fn account(store: &Store, given: Option<&str>) -> Result<String> {
    let accounts = store.list_accounts()?;
    match given {
        Some(g) => accounts
            .iter()
            .find(|a| a.id == g || a.email.eq_ignore_ascii_case(g))
            .map(|a| a.id.clone())
            .ok_or_else(|| anyhow!("unknown account {g:?}")),
        None if accounts.len() == 1 => Ok(accounts[0].id.clone()),
        None => bail!("missing `account:` (one of {})", accounts.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")),
    }
}

/// A mailbox in the account of `message_id`, by id, path or name.
fn mailbox(store: &Store, message_id: &str, given: &str) -> Result<String> {
    let m = store.message_row(message_id)?.ok_or_else(|| anyhow!("unknown message {message_id}"))?;
    let mbs = store.list_mailboxes(Some(&m.summary.account_id))?;
    mbs.iter()
        .find(|b| b.id == given)
        .or_else(|| mbs.iter().find(|b| b.path.eq_ignore_ascii_case(given)))
        .or_else(|| mbs.iter().find(|b| b.name.eq_ignore_ascii_case(given)))
        .map(|b| b.id.clone())
        .ok_or_else(|| anyhow!("unknown mailbox {given:?}"))
}

/// A writable calendar by id or name; may be omitted when there is only one.
fn calendar(store: &Store, given: Option<&str>, account: Option<&str>) -> Result<String> {
    let account_id = account.map(|a| self::account(store, Some(a))).transpose()?;
    let cals: Vec<Calendar> = store.list_calendars(account_id.as_deref())?.into_iter().map(|c| c.cal).filter(|c| !c.read_only).collect();
    match given {
        Some(g) => cals
            .iter()
            .find(|c| c.id == g)
            .or_else(|| cals.iter().find(|c| c.name.eq_ignore_ascii_case(g)))
            .map(|c| c.id.clone())
            .ok_or_else(|| anyhow!("unknown calendar {g:?}")),
        None if cals.len() == 1 => Ok(cals[0].id.clone()),
        None => bail!("missing `calendar:` (one of {})", cals.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")),
    }
}

/// RFC 3339 (`2026-11-02T10:00:00-05:00`) or a date (`2026-11-02`, all day).
fn when(key: &str, v: &str) -> Result<(DateTime<Utc>, bool)> {
    if let Ok(t) = DateTime::parse_from_rfc3339(v) {
        return Ok((t.with_timezone(&Utc), false));
    }
    if let Ok(d) = NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Ok((d.and_hms_opt(0, 0, 0).unwrap().and_utc(), true));
    }
    bail!("{key}: expected a time like 2026-11-02T10:00:00-05:00 or a date like 2026-11-02, got {v:?}")
}

fn apply_event_fields(e: &mut Event, fm: &FrontMatter, body: String) -> Result<()> {
    if let Some(t) = fm.str("title") {
        e.title = t.to_string();
    }
    if let Some(l) = fm.str("location") {
        e.location = l.to_string();
    }
    if let Some(tz) = fm.str("timezone") {
        e.timezone = Some(tz.to_string());
    }
    if let Some(r) = fm.str("rrule") {
        e.rrule = Some(r.to_string());
    }
    let duration = e.end - e.start;
    if let Some(s) = fm.str("start") {
        let (start, all_day) = when("start", s)?;
        e.start = start;
        e.all_day = all_day;
        e.end = start + if all_day { chrono::Duration::days(1) } else if duration > chrono::Duration::zero() { duration } else { chrono::Duration::hours(1) };
    }
    if let Some(s) = fm.str("end") {
        let (end, all_day) = when("end", s)?;
        // An all-day end date is inclusive in the file, exclusive in the event.
        e.end = if all_day { end + chrono::Duration::days(1) } else { end };
    }
    if let Some(a) = fm.bool("all_day")? {
        e.all_day = a;
    }
    if fm.get("attendees").is_some() {
        e.attendees = addresses(fm, "attendees")?
            .into_iter()
            .map(|a| Attendee { email: a.email, name: a.name, status: PartStat::NeedsAction, optional: false })
            .collect();
    }
    if !body.is_empty() {
        e.description = body;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars_lists_and_comments() {
        let (fm, body) = parse(
            "---\naction: archive\nthreads: [a1, \"b, 2\"]\nwhy: 'it''s done' # trailing comment\nto:\n  - Bob <bob@x.test>\n  - carol@x.test\nsubject: \"Re: \\\"hi\\\" # not a comment\"\n---\nBody text\n",
        )
        .unwrap();
        assert_eq!(fm.str("action"), Some("archive"));
        assert_eq!(fm.list("threads"), vec!["a1", "b, 2"]);
        assert_eq!(fm.str("why"), Some("it's done"));
        assert_eq!(fm.list("to"), vec!["Bob <bob@x.test>", "carol@x.test"]);
        assert_eq!(fm.str("subject"), Some("Re: \"hi\" # not a comment"));
        assert_eq!(body, "Body text\n");
    }

    #[test]
    fn requires_front_matter() {
        assert!(parse("action: archive\n").is_err());
        assert!(parse("---\naction: archive\n").is_err());
    }

    #[test]
    fn set_fields_replaces_and_removes_keys() {
        let text = "---\naction: reply\nagent_token: stk_secret\nwarnings:\n  - old\nwhy: ok\n---\nHello\n";
        let out = set_fields(text, &[("agent_token", None), ("warnings", None), ("status", Some("pending".into()))]);
        assert_eq!(out, "---\naction: reply\nwhy: ok\nstatus: pending\n---\nHello\n");
        let again = set_fields(&out, &[("status", Some("rejected".into())), ("note", Some(quote("not now\n\"sorry\"")))]);
        let (fm, body) = parse(&again).unwrap();
        assert_eq!(fm.str("status"), Some("rejected"));
        assert_eq!(fm.str("note"), Some("not now\n\"sorry\""));
        assert_eq!(body, "Hello\n");
    }

    #[test]
    fn event_times_and_all_day_dates() {
        let (t, all_day) = when("start", "2026-11-02T10:00:00-05:00").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-11-02T15:00:00+00:00");
        assert!(!all_day);
        assert!(when("start", "2026-11-02").unwrap().1);
        assert!(when("start", "next tuesday").is_err());
    }

    #[test]
    fn parses_addresses() {
        assert_eq!(address("Bob Smith <bob@x.test>").unwrap(), Address { name: Some("Bob Smith".into()), email: "bob@x.test".into() });
        assert_eq!(address("bob@x.test").unwrap(), Address::new("bob@x.test"));
        assert!(address("bob").is_err());
    }
}
