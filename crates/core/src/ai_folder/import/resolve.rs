//! Turning what a file names (threads, addresses, accounts, mailboxes,
//! calendars, times) into the store's ids and values.

use super::*;

/// A reply to the last message of `thread` (or to `message:`), addressed to
/// its sender unless `to:` says otherwise.
pub(crate) fn reply(store: &Store, fm: &FrontMatter, threads: &[String], body: String) -> Result<Draft> {
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
pub(crate) fn address(s: &str) -> Result<Address> {
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

pub(crate) fn addresses(fm: &FrontMatter, key: &str) -> Result<Vec<Address>> {
    fm.list(key).iter().map(|s| address(s).with_context(|| key.to_string())).collect()
}

/// An account by id or email; may be omitted when there is only one.
pub(crate) fn account(store: &Store, given: Option<&str>) -> Result<String> {
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
pub(crate) fn mailbox(store: &Store, message_id: &str, given: &str) -> Result<String> {
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
pub(crate) fn calendar(store: &Store, given: Option<&str>, account: Option<&str>) -> Result<String> {
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
pub(crate) fn when(key: &str, v: &str) -> Result<(DateTime<Utc>, bool)> {
    if let Ok(t) = DateTime::parse_from_rfc3339(v) {
        return Ok((t.with_timezone(&Utc), false));
    }
    if let Ok(d) = NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Ok((d.and_hms_opt(0, 0, 0).unwrap().and_utc(), true));
    }
    bail!("{key}: expected a time like 2026-11-02T10:00:00-05:00 or a date like 2026-11-02, got {v:?}")
}

pub(crate) fn apply_event_fields(e: &mut Event, fm: &FrontMatter, body: String) -> Result<()> {
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
