//! iCalendar (RFC 5545) parsing and serialization for the subset Satchel
//! needs, plus recurrence expansion and iMIP reply generation.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use satchel_api::{Address, Attendee, Event, EventStatus, PartStat};

/// One property line, after unfolding.
#[derive(Debug, Clone)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, k: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

fn unfold(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for raw in s.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

fn parse_line(line: &str) -> Option<Prop> {
    // Split name;params:value — colons may appear inside quoted params.
    let mut in_quote = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ':' if !in_quote => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = split_unquoted(head, ';');
    let name = parts.remove(0).to_ascii_uppercase();
    let params = parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        })
        .collect();
    Some(Prop { name, params, value: value.to_string() })
}

fn split_unquoted(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q = false;
    for c in s.chars() {
        if c == '"' {
            q = !q;
        }
        if c == sep && !q {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

/// A date-time value as written: UTC, zoned, floating, or all-day date.
#[derive(Debug, Clone, PartialEq)]
pub enum IcalTime {
    Utc(DateTime<Utc>),
    Zoned(NaiveDateTime, String),
    Floating(NaiveDateTime),
    Date(NaiveDate),
}

impl IcalTime {
    pub fn to_utc(&self) -> DateTime<Utc> {
        match self {
            IcalTime::Utc(d) => *d,
            IcalTime::Zoned(n, tz) => match tz.parse::<Tz>() {
                Ok(z) => z
                    .from_local_datetime(n)
                    .earliest()
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|| Utc.from_utc_datetime(n)),
                Err(_) => Utc.from_utc_datetime(n),
            },
            IcalTime::Floating(n) => Utc.from_utc_datetime(n),
            IcalTime::Date(d) => Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()),
        }
    }
    pub fn is_date(&self) -> bool {
        matches!(self, IcalTime::Date(_))
    }
    pub fn tzid(&self) -> Option<&str> {
        match self {
            IcalTime::Zoned(_, t) => Some(t),
            _ => None,
        }
    }
}

fn parse_time(p: &Prop) -> Option<IcalTime> {
    let v = p.value.trim();
    if p.param("VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
        return NaiveDate::parse_from_str(v, "%Y%m%d").ok().map(IcalTime::Date);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some(IcalTime::Utc(Utc.from_utc_datetime(&n)));
    }
    let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    match p.param("TZID") {
        Some(tz) => Some(IcalTime::Zoned(n, normalize_tzid(tz))),
        None => Some(IcalTime::Floating(n)),
    }
}

/// Map common Windows/Outlook zone names and "/mozilla.org/..." prefixes to IANA.
fn normalize_tzid(tz: &str) -> String {
    let t = tz.trim().trim_start_matches('/');
    let t = t.rsplit_once("mozilla.org/20070129_1/").map(|(_, r)| r).unwrap_or(t);
    let mapped = match t {
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" => "America/Chicago",
        "Mountain Standard Time" => "America/Denver",
        "Pacific Standard Time" => "America/Los_Angeles",
        "GMT Standard Time" => "Europe/London",
        "W. Europe Standard Time" => "Europe/Berlin",
        "Romance Standard Time" => "Europe/Paris",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "India Standard Time" => "Asia/Kolkata",
        "China Standard Time" => "Asia/Shanghai",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "UTC" | "Coordinated Universal Time" => "UTC",
        other => other,
    };
    mapped.to_string()
}

fn parse_duration(s: &str) -> Option<Duration> {
    // PnW / PnDTnHnMnS, optional sign.
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => in_time = true,
            'W' => total += Duration::weeks(num.parse().ok()?),
            'D' => total += Duration::days(num.parse().ok()?),
            'H' if in_time => total += Duration::hours(num.parse().ok()?),
            'M' if in_time => total += Duration::minutes(num.parse().ok()?),
            'S' if in_time => total += Duration::seconds(num.parse().ok()?),
            _ => return None,
        }
        if !c.is_ascii_digit() && c != 'T' {
            num.clear();
        }
    }
    Some(if neg { -total } else { total })
}

fn mailto(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v)
        .to_string()
}

fn partstat(s: Option<&str>) -> PartStat {
    match s.map(|s| s.to_ascii_uppercase()).as_deref() {
        Some("ACCEPTED") => PartStat::Accepted,
        Some("DECLINED") => PartStat::Declined,
        Some("TENTATIVE") => PartStat::Tentative,
        _ => PartStat::NeedsAction,
    }
}

pub fn partstat_str(p: PartStat) -> &'static str {
    match p {
        PartStat::Accepted => "ACCEPTED",
        PartStat::Declined => "DECLINED",
        PartStat::Tentative => "TENTATIVE",
        PartStat::NeedsAction => "NEEDS-ACTION",
    }
}

#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: IcalTime,
    pub end: Option<IcalTime>,
    pub duration: Option<Duration>,
    pub rrule: Option<String>,
    pub exdates: Vec<IcalTime>,
    pub recurrence_id: Option<IcalTime>,
    pub status: EventStatus,
    pub organizer: Option<Address>,
    pub attendees: Vec<Attendee>,
    pub sequence: i64,
}

impl ParsedEvent {
    pub fn start_utc(&self) -> DateTime<Utc> {
        self.start.to_utc()
    }
    pub fn end_utc(&self) -> DateTime<Utc> {
        if let Some(e) = &self.end {
            return e.to_utc();
        }
        if let Some(d) = self.duration {
            return self.start_utc() + d;
        }
        if self.start.is_date() {
            self.start_utc() + Duration::days(1)
        } else {
            self.start_utc()
        }
    }
    pub fn duration(&self) -> Duration {
        self.end_utc() - self.start_utc()
    }

    /// Convert into the API model. `owner_email` determines `my_status`.
    pub fn to_event(&self, calendar_id: &str, owner_email: Option<&str>) -> Event {
        let my_status = owner_email.and_then(|me| {
            self.attendees
                .iter()
                .find(|a| a.email.eq_ignore_ascii_case(me))
                .map(|a| a.status)
        });
        Event {
            id: None,
            calendar_id: calendar_id.to_string(),
            uid: Some(self.uid.clone()),
            title: self.summary.clone(),
            description: self.description.clone(),
            location: self.location.clone(),
            start: self.start_utc(),
            end: self.end_utc(),
            all_day: self.start.is_date(),
            timezone: self.start.tzid().map(|s| s.to_string()),
            rrule: self.rrule.clone(),
            attendees: self.attendees.clone(),
            organizer: self.organizer.clone(),
            status: self.status,
            my_status,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParsedCalendar {
    pub method: Option<String>,
    pub events: Vec<ParsedEvent>,
}

pub fn parse(ics: &str) -> ParsedCalendar {
    let mut cal = ParsedCalendar::default();
    let mut cur: Option<ParsedEvent> = None;
    let mut depth_other = 0; // inside VALARM etc. within VEVENT
    for line in unfold(ics) {
        let Some(p) = parse_line(&line) else { continue };
        match (p.name.as_str(), p.value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => {
                cur = Some(ParsedEvent {
                    uid: String::new(),
                    summary: String::new(),
                    description: String::new(),
                    location: String::new(),
                    start: IcalTime::Utc(Utc::now()),
                    end: None,
                    duration: None,
                    rrule: None,
                    exdates: vec![],
                    recurrence_id: None,
                    status: EventStatus::Confirmed,
                    organizer: None,
                    attendees: vec![],
                    sequence: 0,
                });
                continue;
            }
            ("END", "VEVENT") => {
                if let Some(e) = cur.take() {
                    cal.events.push(e);
                }
                continue;
            }
            ("BEGIN", _) if cur.is_some() => {
                depth_other += 1;
                continue;
            }
            ("END", _) if cur.is_some() && depth_other > 0 => {
                depth_other -= 1;
                continue;
            }
            ("METHOD", _) if cur.is_none() => {
                cal.method = Some(p.value.trim().to_ascii_uppercase());
                continue;
            }
            _ => {}
        }
        if depth_other > 0 {
            continue;
        }
        let Some(e) = cur.as_mut() else { continue };
        match p.name.as_str() {
            "UID" => e.uid = p.value.trim().to_string(),
            "SUMMARY" => e.summary = unescape(&p.value),
            "DESCRIPTION" => e.description = unescape(&p.value),
            "LOCATION" => e.location = unescape(&p.value),
            "DTSTART" => {
                if let Some(t) = parse_time(&p) {
                    e.start = t
                }
            }
            "DTEND" => e.end = parse_time(&p),
            "DURATION" => e.duration = parse_duration(p.value.trim()),
            "RRULE" => e.rrule = Some(p.value.trim().to_string()),
            "EXDATE" => {
                for v in p.value.split(',') {
                    let q = Prop { value: v.to_string(), ..p.clone() };
                    if let Some(t) = parse_time(&q) {
                        e.exdates.push(t);
                    }
                }
            }
            "RECURRENCE-ID" => e.recurrence_id = parse_time(&p),
            "SEQUENCE" => e.sequence = p.value.trim().parse().unwrap_or(0),
            "STATUS" => {
                e.status = match p.value.trim().to_ascii_uppercase().as_str() {
                    "TENTATIVE" => EventStatus::Tentative,
                    "CANCELLED" => EventStatus::Cancelled,
                    _ => EventStatus::Confirmed,
                }
            }
            "ORGANIZER" => {
                e.organizer = Some(Address { name: p.param("CN").map(|s| s.to_string()), email: mailto(&p.value) })
            }
            "ATTENDEE" => e.attendees.push(Attendee {
                email: mailto(&p.value),
                name: p.param("CN").map(|s| s.to_string()),
                status: partstat(p.param("PARTSTAT")),
                optional: p.param("ROLE").is_some_and(|r| r.eq_ignore_ascii_case("OPT-PARTICIPANT")),
            }),
            _ => {}
        }
    }
    cal
}

fn fmt_utc(d: &DateTime<Utc>) -> String {
    d.format("%Y%m%dT%H%M%SZ").to_string()
}

fn fold(line: &str) -> String {
    // Fold at 74 octets on char boundaries.
    let mut out = String::new();
    let mut count = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if count + len > 74 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(c);
        count += len;
    }
    out.push_str("\r\n");
    out
}

fn time_prop(name: &str, t: &DateTime<Utc>, all_day: bool, tz: Option<&str>) -> String {
    if all_day {
        return format!("{name};VALUE=DATE:{}", t.format("%Y%m%d"));
    }
    if let Some(z) = tz.and_then(|z| z.parse::<Tz>().ok())
        && z != chrono_tz::UTC {
            return format!("{name};TZID={}:{}", z.name(), t.with_timezone(&z).format("%Y%m%dT%H%M%S"));
        }
    format!("{name}:{}", fmt_utc(t))
}

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
        "PRODID:-//Satchel Mail//EN".into(),
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
        "PRODID:-//Satchel Mail//EN".into(),
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
        Some(rule) => expand_rrule(master, rule, from - dur, to),
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
        let e = s + dur;
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
        if oe > from && os < to && rid_in_series.is_some_and(|r| r < from - dur || r >= to) {
            out.push((os, oe, Some(i)));
        }
    }
    out.sort_by_key(|x| x.0);
    out
}

fn expand_rrule(master: &ParsedEvent, rule: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateTime<Utc>> {
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

fn normalize_until(rule: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:abc\r\nDTSTART;TZID=America/New_York:20260105T090000\r\nDTEND;TZID=America/New_York:20260105T093000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=10\r\nEXDATE;TZID=America/New_York:20260112T090000\r\nSUMMARY:Stand\r\n  up\r\nORGANIZER;CN=Bob:mailto:bob@x.test\r\nATTENDEE;CN=\"Alice, A\";PARTSTAT=NEEDS-ACTION:mailto:alice@x.test\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:abc\r\nRECURRENCE-ID;TZID=America/New_York:20260119T090000\r\nDTSTART;TZID=America/New_York:20260119T110000\r\nDTEND;TZID=America/New_York:20260119T113000\r\nSUMMARY:Moved\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parse_and_expand() {
        let c = parse(ICS);
        assert_eq!(c.method.as_deref(), Some("REQUEST"));
        assert_eq!(c.events.len(), 2);
        let m = &c.events[0];
        assert_eq!(m.summary, "Stand up");
        assert_eq!(m.attendees[0].name.as_deref(), Some("Alice, A"));
        assert_eq!(m.start_utc().to_rfc3339(), "2026-01-05T14:00:00+00:00");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
        let occ = expand(m, &c.events[1..], from, to);
        // Jan 5, (12 excluded), 19 moved, 26
        assert_eq!(occ.len(), 3);
        assert_eq!(occ[1].0.to_rfc3339(), "2026-01-19T16:00:00+00:00");
        assert_eq!(occ[1].2, Some(0));
    }

    #[test]
    fn roundtrip() {
        let c = parse(ICS);
        let e = c.events[0].to_event("cal", Some("alice@x.test"));
        assert_eq!(e.my_status, Some(PartStat::NeedsAction));
        let s = serialize(&e, &SerializeOpts::default());
        let back = parse(&s);
        assert_eq!(back.events[0].summary, "Stand up");
        assert_eq!(back.events[0].start_utc(), e.start);
        assert_eq!(back.events[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO;COUNT=10"));
        let r = reply(&c.events[0], "alice@x.test", None, PartStat::Accepted);
        let changed = set_partstat(ICS, "alice@x.test", PartStat::Accepted);
        let back = parse(&changed);
        assert_eq!(back.events[0].attendees[0].status, PartStat::Accepted);
        assert_eq!(back.events[0].attendees[0].name.as_deref(), Some("Alice, A"));
        assert_eq!(override_blocks(ICS).len(), 1);
        let with = serialize(&e, &SerializeOpts { extra_components: &override_blocks(ICS), ..Default::default() });
        assert_eq!(parse(&with).events.len(), 2);
        assert!(r.contains("METHOD:REPLY"));
        assert!(r.contains("PARTSTAT=ACCEPTED"));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1D"), Some(Duration::days(1)));
        assert_eq!(parse_duration("P2W"), Some(Duration::weeks(2)));
    }
}
