//! Recurrence expansion.

use super::*;

/// Expand an event (master + its overrides) into concrete occurrences
/// intersecting [from, to). Returns (start, end, override index or None).
pub fn expand(
    master: &ParsedEvent,
    overrides: &[ParsedEvent],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, Option<usize>)> {
    let dur = master.duration();
    let mut out = vec![];
    let starts: Vec<DateTime<Utc>> = match &master.rrule {
        None => vec![master.start_utc()],
        Some(rule) => expand_rrule(master, rule, from.checked_sub_signed(dur).unwrap_or(from), to),
    };
    let override_at = |s: &DateTime<Utc>| {
        overrides
            .iter()
            .position(|o| o.recurrence_id.as_ref().is_some_and(|r| r.to_utc() == *s))
    };
    let exdates: Vec<DateTime<Utc>> = master.exdates.iter().map(|x| x.to_utc()).collect();
    for s in starts {
        if exdates.contains(&s) {
            continue;
        }
        if let Some(i) = override_at(&s) {
            let o = &overrides[i];
            if o.status == EventStatus::Cancelled {
                continue;
            }
            let (os, oe) = (o.start_utc(), o.end_utc());
            if oe > from && os < to {
                out.push((os, oe, Some(i)));
            }
            continue;
        }
        let e = s.checked_add_signed(dur).unwrap_or(s);
        if e > from && s < to || (dur.is_zero() && s >= from && s < to) {
            out.push((s, e, None));
        }
    }
    // Overrides moved into the window from outside the series' range.
    for (i, o) in overrides.iter().enumerate() {
        if out.iter().any(|(_, _, idx)| *idx == Some(i)) || o.status == EventStatus::Cancelled {
            continue;
        }
        let (os, oe) = (o.start_utc(), o.end_utc());
        let rid_in_series = o.recurrence_id.as_ref().map(|r| r.to_utc());
        let series_from = from.checked_sub_signed(dur).unwrap_or(from);
        if oe > from && os < to && rid_in_series.is_some_and(|r| r < series_from || r >= to) {
            out.push((os, oe, Some(i)));
        }
    }
    out.sort_by_key(|x| x.0);
    out
}

pub(crate) fn expand_rrule(master: &ParsedEvent, rule: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let dtstart = match &master.start {
        IcalTime::Utc(d) => format!("DTSTART:{}", fmt_utc(d)),
        IcalTime::Zoned(n, tz) if tz.parse::<Tz>().is_ok() => {
            format!("DTSTART;TZID={tz}:{}", n.format("%Y%m%dT%H%M%S"))
        }
        IcalTime::Zoned(n, _) | IcalTime::Floating(n) => {
            format!("DTSTART:{}", Utc.from_utc_datetime(n).format("%Y%m%dT%H%M%SZ"))
        }
        IcalTime::Date(d) => format!("DTSTART:{}T000000Z", d.format("%Y%m%d")),
    };
    // rrule requires UNTIL in UTC when DTSTART is zoned; normalize date-only UNTIL.
    let rule = normalize_until(rule);
    let text = format!("{dtstart}\nRRULE:{rule}");
    let set: rrule::RRuleSet = match text.parse() {
        Ok(s) => s,
        Err(_) => return vec![master.start_utc()],
    };
    let tz = rrule::Tz::UTC;
    let after = tz.from_utc_datetime(&from.naive_utc());
    let before = tz.from_utc_datetime(&to.naive_utc());
    set.after(after)
        .before(before)
        .all(2000)
        .dates
        .into_iter()
        .map(|d| d.with_timezone(&Utc))
        .collect()
}

pub(crate) fn normalize_until(rule: &str) -> String {
    rule.split(';')
        .map(|part| {
            if let Some(v) = part.strip_prefix("UNTIL=") {
                if v.len() == 8 {
                    return format!("UNTIL={v}T235959Z");
                }
                if !v.ends_with('Z') && v.len() == 15 {
                    return format!("UNTIL={v}Z");
                }
            }
            part.to_string()
        })
        .collect::<Vec<_>>()
        .join(";")
}
