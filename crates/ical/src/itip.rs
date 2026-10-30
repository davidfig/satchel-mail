//! Invitations (iMIP): overrides, PARTSTAT changes and replies.

use super::*;

/// Raw text of VEVENT components that carry a RECURRENCE-ID (overrides).
pub fn override_blocks(ics: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur: Option<Vec<&str>> = None;
    for line in ics.split('\n') {
        let l = line.trim_end_matches('\r');
        if l.eq_ignore_ascii_case("BEGIN:VEVENT") {
            cur = Some(vec![l]);
            continue;
        }
        if let Some(c) = cur.as_mut() {
            c.push(l);
            if l.eq_ignore_ascii_case("END:VEVENT") {
                let block = cur.take().unwrap();
                if block.iter().any(|x| x.to_ascii_uppercase().starts_with("RECURRENCE-ID")) {
                    out.push(block.join("\r\n"));
                }
            }
        }
    }
    out
}

/// Set the PARTSTAT of `email` on every ATTENDEE line (all components).
pub fn set_partstat(ics: &str, email: &str, status: PartStat) -> String {
    let target = format!("mailto:{}", email.to_ascii_lowercase());
    let mut out = String::new();
    for line in unfold(ics) {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("ATTENDEE") && line.to_ascii_lowercase().ends_with(&target)
            && let Some(p) = parse_line(&line) {
                let mut l = String::from("ATTENDEE");
                let mut had = false;
                for (k, v) in &p.params {
                    if k == "PARTSTAT" {
                        l.push_str(&format!(";PARTSTAT={}", partstat_str(status)));
                        had = true;
                    } else if k == "RSVP" {
                        l.push_str(";RSVP=FALSE");
                    } else if v.contains([':', ';', ',']) || v.contains(' ') {
                        l.push_str(&format!(";{k}=\"{v}\""));
                    } else {
                        l.push_str(&format!(";{k}={v}"));
                    }
                }
                if !had {
                    l.push_str(&format!(";PARTSTAT={}", partstat_str(status)));
                }
                l.push(':');
                l.push_str(&p.value);
                out.push_str(&fold(&l));
                continue;
            }
        out.push_str(&fold(&line));
    }
    out
}

/// iMIP REPLY for responding to an invitation (RFC 6047).
pub fn reply(original: &ParsedEvent, attendee_email: &str, attendee_name: Option<&str>, response: PartStat) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//satchel.email//EN".into(),
        "METHOD:REPLY".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", original.uid),
        format!("DTSTAMP:{}", fmt_utc(&Utc::now())),
        format!("SEQUENCE:{}", original.sequence),
        time_prop("DTSTART", &original.start_utc(), original.start.is_date(), original.start.tzid()),
        time_prop("DTEND", &original.end_utc(), original.start.is_date(), original.start.tzid()),
        format!("SUMMARY:{}", escape(&original.summary)),
    ];
    if let Some(r) = &original.recurrence_id {
        lines.push(time_prop("RECURRENCE-ID", &r.to_utc(), r.is_date(), r.tzid()));
    }
    if let Some(o) = &original.organizer {
        lines.push(format!("ORGANIZER:mailto:{}", o.email));
    }
    let cn = attendee_name.map(|n| format!(";CN=\"{}\"", n.replace('"', ""))).unwrap_or_default();
    lines.push(format!("ATTENDEE{cn};PARTSTAT={}:mailto:{attendee_email}", partstat_str(response)));
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    lines.iter().map(|l| fold(l)).collect()
}

pub fn new_uid() -> String {
    format!("{}@satchel", uuid::Uuid::new_v4())
}
