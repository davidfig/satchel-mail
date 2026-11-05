//! Invitations that arrive by email.


use anyhow::Result;
use base64::Engine as _;
use chrono::Utc;
use satchel_api::*;
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::error::{invalid, not_found};
use crate::store::new_id;

impl Engine {
    /// The invitation in a parsed message, if any. When it is about an event
    /// already on the calendar, the stored event's title is shown: the email's
    /// own SUMMARY is whatever its sender wrote.
    pub(crate) fn invite_from(&self, account_id: &str, p: &satchel_mime::Parsed) -> Option<Invite> {
        let ics = p.calendar_parts.first()?;
        let cal = satchel_ical::parse(ics);
        let e = cal.events.first()?;
        let me = self.store.get_account(account_id).ok().flatten()?.email;
        let sender = p.from.as_ref().map(|f| f.email.as_str()).unwrap_or_default();
        let existing = self.invited_event(account_id, e, sender, crate::mail::spam::not_forged(p)).ok().flatten();
        let event_id = existing.as_ref().and_then(|ev| ev.id.clone());
        let my_status = match &existing {
            Some(ev) => ev.my_status,
            None => e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(&me)).map(|a| a.status),
        };
        Some(Invite {
            uid: e.uid.clone(),
            method: cal.method.clone().unwrap_or_else(|| "PUBLISH".into()),
            title: existing.map(|ev| ev.title).unwrap_or_else(|| e.summary.clone()),
            start: e.start_utc(),
            end: e.end_utc(),
            all_day: e.start.is_date(),
            location: e.location.clone(),
            organizer: e.organizer.clone(),
            my_status,
            event_id,
        })
    }

    /// The synced event an invitation is about: same UID *and* same
    /// organizer, sent by that organizer, and not failing sender checks.
    /// Anyone who learns a meeting's UID can send an invitation with it; one
    /// from anyone else is a different invitation and must not act on (or
    /// show the status of) the real meeting.
    fn invited_event(&self, account_id: &str, e: &satchel_ical::ParsedEvent, sender: &str, not_forged: bool) -> Result<Option<Event>> {
        let Some(org) = e.organizer.as_ref().map(|o| o.email.trim().to_string()) else { return Ok(None) };
        if !not_forged || !org.eq_ignore_ascii_case(sender.trim()) {
            return Ok(None);
        }
        for stored in self.store.events_by_uid(account_id, &e.uid)? {
            if let Some(ev) = crate::calendar::get_event(&self.store, &stored.id)?
                && ev.organizer.as_ref().is_some_and(|o| o.email.trim().eq_ignore_ascii_case(&org))
            {
                return Ok(Some(ev));
            }
        }
        Ok(None)
    }

    /// RSVP to an emailed invitation. If the event is already in a synced
    /// calendar, respond there (the server notifies the organizer). Otherwise add
    /// it to the account's first writable calendar and email an iMIP REPLY.
    pub(crate) async fn respond_email_invite(&self, message_id: &str, response: PartStat) -> Result<Value> {
        let m = self.store.message_row(message_id)?.ok_or_else(|| not_found("message"))?;
        let acc = self.store.get_account(&m.summary.account_id)?.ok_or_else(|| not_found("account"))?;
        let raw = self.raw_for(message_id).await?.ok_or_else(|| not_found("message body"))?;
        let parsed = satchel_mime::parse(&raw).unwrap_or_default();
        let ics = parsed.calendar_parts.first().ok_or_else(|| invalid("this message has no invitation"))?;
        let cal = satchel_ical::parse(ics);
        let ev = cal.events.first().ok_or_else(|| invalid("invitation has no event"))?.clone();
        let from = m.summary.from.as_ref().map(|f| f.email.trim().to_string()).unwrap_or_default();
        let not_forged = crate::mail::spam::not_forged(&parsed);
        if let Some(existing) = self.invited_event(&acc.id, &ev, &from, not_forged)?.and_then(|e| e.id) {
            self.respond(&existing, response, "user")?;
            return Ok(json!({ "ok": true, "via": "calendar", "event_id": existing }));
        }
        if !not_forged {
            return Err(invalid("This invitation failed the sender checks, so it may not really be from its organizer. Satchel won't send your reply."));
        }
        // The reply goes to the ORGANIZER, which the invitation's sender wrote:
        // only email it when the sender is that organizer, so an invitation
        // can't make the user send mail to someone else.
        if let Some(org) = &ev.organizer
            && !org.email.trim().eq_ignore_ascii_case(&from)
        {
            return Err(invalid(format!(
                "This invitation was sent by {} but names {} as the organizer, so Satchel won't send your reply. If you know the organizer, reply to them directly.",
                if from.is_empty() { "an unknown sender" } else { &from },
                org.email
            )));
        }
        // Not synced: add a copy and reply by email. Outlook can't hold a copy
        // of someone else's meeting without re-inviting its attendees; it adds
        // the invitation to the calendar itself, so only reply there. A copy
        // never shares a UID with an event already on the calendar (one from
        // another organizer), so it can't overwrite it.
        let can_copy = !matches!(acc.config, AccountConfig::Graph { .. }) && self.store.events_by_uid(&acc.id, &ev.uid)?.is_empty();
        let target = self.store.list_calendars(Some(&acc.id))?.into_iter().filter(|_| can_copy).find(|c| !c.cal.read_only);
        let mut event_id = None;
        if let Some(c) = target {
            let with_status = satchel_ical::set_partstat(&invite_copy(ics, &ev.uid), &acc.email, response);
            let body: String = {
                let p = satchel_ical::parse(&with_status);
                let range = crate::calendar::series_range(&p.events);
                let id = self.store.upsert_event(&c.cal.id, &acc.id, &format!("local:{}", new_id()), None, &ev.uid, &with_status, &ev.summary, ev.location.trim(), range, ev.rrule.is_some(), true)?;
                event_id = Some(id.clone());
                id
            };
            self.enqueue(&acc.id, "cal_put", json!({ "event_id": body }), "user")?;
            self.emit(CoreEvent::CalendarChanged { account_id: acc.id.clone() });
        }
        if let Some(org) = &ev.organizer {
            let reply = satchel_ical::reply(&ev, &acc.email, Some(&acc.display_name), response);
            let verb = match response {
                PartStat::Accepted => "Accepted",
                PartStat::Declined => "Declined",
                PartStat::Tentative => "Tentative",
                PartStat::NeedsAction => "Updated",
            };
            let o = satchel_mime::Outgoing {
                from: Address { name: Some(acc.display_name.clone()).filter(|s| !s.is_empty()), email: acc.email.clone() },
                to: vec![org.clone()],
                subject: format!("{verb}: {}", ev.summary),
                text: format!("{} has {} this invitation.", if acc.display_name.is_empty() { &acc.email } else { &acc.display_name }, verb.to_lowercase()),
                attachments: vec![("invite.ics".into(), "text/calendar; method=REPLY; charset=utf-8".into(), reply.into_bytes())],
                message_id: satchel_mime::new_message_id(acc.email.split('@').nth(1).unwrap_or("satchel.local")),
                date: Utc::now(),
                ..Default::default()
            };
            let raw = satchel_mime::build(&o);
            self.enqueue(
                &acc.id,
                "send",
                json!({ "raw": base64::engine::general_purpose::STANDARD.encode(&raw), "rcpts": [org.email], "from": acc.email, "subject": o.subject }),
                "user",
            )?;
        }
        self.store.audit("user", "invite_response", &json!({ "event": ev.summary, "response": satchel_ical::partstat_str(response), "via": "email" }))?;
        Ok(json!({ "ok": true, "via": "email", "event_id": event_id }))
    }
}

/// What to keep of an invitation in the user's calendar: the calendar's own
/// properties (without METHOD, which stored objects must not carry), its time
/// zones, and the invited series only (the VEVENTs with its UID: master and
/// overrides), without alarms. Anything else in the sender's file (other
/// events, to-dos, alarms that would fire on the user's devices) is dropped.
/// Works on unfolded lines, so folding can't hide a BEGIN from the filter.
fn invite_copy(ics: &str, uid: &str) -> String {
    let lines = satchel_ical::unfold(ics);
    let mut out: Vec<&str> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let (name, value) = name_value(&lines[i]);
        if name == "BEGIN" && value != "VCALENDAR" {
            let end = component_end(&lines, i);
            let block = &lines[i..=end];
            match value.as_str() {
                "VTIMEZONE" => out.extend(only_subcomponents(block, &["STANDARD", "DAYLIGHT"])),
                "VEVENT" if prop_value(block, "UID").as_deref() == Some(uid) => out.extend(only_subcomponents(block, &[])),
                _ => {}
            }
            i = end + 1;
            continue;
        }
        if name != "METHOD" {
            out.push(&lines[i]);
        }
        i += 1;
    }
    out.into_iter().map(satchel_ical::fold).collect()
}

/// An unfolded line's property name and, for BEGIN/END, its value, both
/// uppercase. Parameters (`BEGIN;X=Y:VALARM`) are skipped.
fn name_value(line: &str) -> (String, String) {
    let name_end = line.find([':', ';']).unwrap_or(line.len());
    let name = line[..name_end].trim().to_ascii_uppercase();
    if name != "BEGIN" && name != "END" {
        return (name, String::new());
    }
    // The value starts after the first colon outside quoted parameters.
    let mut quoted = false;
    let colon = line.char_indices().find(|&(_, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        c == ':' && !quoted
    });
    let value = colon.map(|(j, _)| line[j + 1..].trim().to_ascii_uppercase()).unwrap_or_default();
    (name, value)
}

/// Index of the END line that closes the component opened at `start`.
fn component_end(lines: &[String], start: usize) -> usize {
    let mut depth = 0;
    for (j, l) in lines.iter().enumerate().skip(start) {
        match name_value(l).0.as_str() {
            "BEGIN" => depth += 1,
            "END" => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    lines.len() - 1
}

/// A component's lines, keeping only the nested components named in `keep`
/// (with everything inside them); others, such as VALARM, are dropped.
fn only_subcomponents<'a>(block: &'a [String], keep: &[&str]) -> Vec<&'a str> {
    let mut out = vec![];
    let mut j = 0;
    while j < block.len() {
        let (name, value) = name_value(&block[j]);
        if j > 0 && j < block.len() - 1 && name == "BEGIN" {
            let end = j + component_end(&block[j..], 0);
            if keep.contains(&value.as_str()) {
                out.extend(block[j..=end].iter().map(String::as_str));
            }
            j = end + 1;
            continue;
        }
        out.push(&block[j]);
        j += 1;
    }
    out
}

/// The value of a component's own property (not a nested one's).
fn prop_value(block: &[String], name: &str) -> Option<String> {
    let mut depth = 0;
    for l in block {
        match name_value(l).0.as_str() {
            "BEGIN" => depth += 1,
            "END" => depth -= 1,
            n if depth == 1 && n == name => return Some(l.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_only_the_invited_series() {
        let ics = [
            "BEGIN:VCALENDAR", "VERSION:2.0", "METHOD:REQUEST", "BEGIN:VTIMEZONE", "TZID:X", "END:VTIMEZONE",
            "BEGIN:VEVENT", "UID:a", " bc", "SUMMARY:Real", "BEGIN:VALARM", "ACTION:DISPLAY", "END:VALARM", "END:VEVENT",
            "BEGIN:VEVENT", "UID:abc", "RECURRENCE-ID:20261101T150000Z", "SUMMARY:Moved", "END:VEVENT",
            "BEGIN:VEVENT", "UID:other", "SUMMARY:Smuggled", "END:VEVENT", "BEGIN:VTODO", "UID:abc", "END:VTODO", "END:VCALENDAR", "",
        ]
        .join("\r\n");
        let copy = invite_copy(&ics, "abc");
        assert!(!copy.contains("METHOD") && !copy.contains("VALARM") && !copy.contains("Smuggled") && !copy.contains("VTODO"), "{copy}");
        assert!(copy.contains("TZID:X") && copy.contains("SUMMARY:Real") && copy.contains("SUMMARY:Moved") && copy.ends_with("END:VCALENDAR\r\n"), "{copy}");
        assert_eq!(satchel_ical::parse(&copy).events.len(), 2);
    }

    #[test]
    fn folding_and_parameters_dont_hide_components() {
        let ics = [
            "BEGIN:VCALENDAR", "METH", " OD:REQUEST", "BEGIN:VTIMEZONE", "TZID:X", "BEGIN:STANDARD", "TZOFFSETTO:+0000", "END:STANDARD",
            "BEG", " IN:VALARM", "END:VALARM", "END:VTIMEZONE",
            "BEGIN:VEVENT", "UID:abc", "SUMMARY:Real", "BEG", " IN:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT5M", "END:VALARM",
            "BEGIN;X-Y=\"a:b\":VALARM", "ACTION:AUDIO", "END:VALARM", "END:VEVENT",
            "BEG", " IN:VEVENT", "UID:other", "SUMMARY:Smuggled", "END:VEVENT", "END:VCALENDAR", "",
        ]
        .join("\r\n");
        let copy = invite_copy(&ics, "abc");
        assert!(!copy.contains("METHOD") && !copy.contains("OD:REQUEST") && !copy.contains("VALARM") && !copy.contains("ACTION") && !copy.contains("Smuggled"), "{copy}");
        assert!(copy.contains("BEGIN:STANDARD") && copy.contains("SUMMARY:Real") && copy.ends_with("END:VCALENDAR\r\n"), "{copy}");
        assert_eq!(satchel_ical::parse(&copy).events.len(), 1);
    }
}
