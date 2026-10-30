//! Writing events as iCalendar.

use super::*;

#[derive(Default)]
pub struct SerializeOpts<'a> {
    pub method: Option<&'a str>,
    pub sequence: i64,
    /// EXDATEs to keep from the existing master.
    pub exdates: &'a [DateTime<Utc>],
    /// Raw VEVENT blocks (overrides) to carry over unchanged.
    pub extra_components: &'a [String],
}


/// Serialize an API event as a complete VCALENDAR object.
pub fn serialize(e: &Event, opts: &SerializeOpts) -> String {
    let uid = e.uid.clone().unwrap_or_else(new_uid);
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//satchel.email//EN".into(),
        "CALSCALE:GREGORIAN".into(),
    ];
    if let Some(m) = opts.method {
        lines.push(format!("METHOD:{m}"));
    }
    lines.push("BEGIN:VEVENT".into());
    lines.push(format!("UID:{uid}"));
    lines.push(format!("DTSTAMP:{}", fmt_utc(&Utc::now())));
    lines.push(format!("SEQUENCE:{}", opts.sequence));
    lines.push(time_prop("DTSTART", &e.start, e.all_day, e.timezone.as_deref()));
    lines.push(time_prop("DTEND", &e.end, e.all_day, e.timezone.as_deref()));
    lines.push(format!("SUMMARY:{}", escape(&e.title)));
    if !e.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&e.description)));
    }
    if !e.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape(&e.location)));
    }
    if let Some(r) = &e.rrule
        && !r.trim().is_empty() {
            lines.push(format!("RRULE:{}", r.trim().trim_start_matches("RRULE:")));
        }
    for x in opts.exdates {
        lines.push(time_prop("EXDATE", x, e.all_day, e.timezone.as_deref()));
    }
    lines.push(format!(
        "STATUS:{}",
        match e.status {
            EventStatus::Confirmed => "CONFIRMED",
            EventStatus::Tentative => "TENTATIVE",
            EventStatus::Cancelled => "CANCELLED",
        }
    ));
    if let Some(o) = &e.organizer {
        lines.push(match &o.name {
            Some(n) => format!("ORGANIZER;CN=\"{}\":mailto:{}", n.replace('"', ""), o.email),
            None => format!("ORGANIZER:mailto:{}", o.email),
        });
    }
    for a in &e.attendees {
        let mut l = String::from("ATTENDEE");
        if let Some(n) = &a.name {
            l.push_str(&format!(";CN=\"{}\"", n.replace('"', "")));
        }
        l.push_str(&format!(
            ";ROLE={};PARTSTAT={};RSVP=TRUE:mailto:{}",
            if a.optional { "OPT-PARTICIPANT" } else { "REQ-PARTICIPANT" },
            partstat_str(a.status),
            a.email
        ));
        lines.push(l);
    }
    lines.push("END:VEVENT".into());
    let mut out: String = lines.iter().map(|l| fold(l)).collect();
    for block in opts.extra_components {
        out.push_str(block.trim_end());
        out.push_str("\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}
