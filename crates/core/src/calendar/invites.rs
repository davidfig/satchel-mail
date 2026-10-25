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
    pub(crate) fn invite_from(&self, account_id: &str, parts: &[String]) -> Option<Invite> {
        let ics = parts.first()?;
        let cal = satchel_ical::parse(ics);
        let e = cal.events.first()?;
        let me = self.store.get_account(account_id).ok().flatten()?.email;
        let event_id = self.store.events_by_uid(account_id, &e.uid).ok().and_then(|v| v.into_iter().next()).map(|x| x.id);
        let my_status = match &event_id {
            Some(id) => crate::calendar::get_event(&self.store, id).ok().flatten().and_then(|ev| ev.my_status),
            None => e.attendees.iter().find(|a| a.email.eq_ignore_ascii_case(&me)).map(|a| a.status),
        };
        Some(Invite {
            uid: e.uid.clone(),
            method: cal.method.clone().unwrap_or_else(|| "PUBLISH".into()),
            title: e.summary.clone(),
            start: e.start_utc(),
            end: e.end_utc(),
            all_day: e.start.is_date(),
            location: e.location.clone(),
            organizer: e.organizer.clone(),
            my_status,
            event_id,
        })
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
        if let Some(existing) = self.store.events_by_uid(&acc.id, &ev.uid)?.into_iter().next() {
            self.respond(&existing.id, response, "user")?;
            return Ok(json!({ "ok": true, "via": "calendar", "event_id": existing.id }));
        }
        // Not synced: add a copy and reply by email. Outlook can't hold a copy
        // of someone else's meeting without re-inviting its attendees; it adds
        // the invitation to the calendar itself, so only reply there.
        let can_copy = !matches!(acc.config, AccountConfig::Graph { .. });
        let target = self.store.list_calendars(Some(&acc.id))?.into_iter().filter(|_| can_copy).find(|c| !c.cal.read_only);
        let mut event_id = None;
        if let Some(c) = target {
            let with_status = satchel_ical::set_partstat(ics, &acc.email, response);
            let body: String = {
                let p = satchel_ical::parse(&with_status);
                let range = crate::calendar::series_range(&p.events);
                // Strip METHOD: stored objects must not carry it.
                let clean: String = with_status.lines().filter(|l| !l.starts_with("METHOD:")).map(|l| format!("{}\n", l.trim_end_matches('\r'))).collect();
                let id = self.store.upsert_event(&c.cal.id, &acc.id, &format!("local:{}", new_id()), None, &ev.uid, &clean, &ev.summary, range, ev.rrule.is_some(), true)?;
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
