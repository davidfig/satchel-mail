//! Parsing VEVENTs into events.

use super::*;

pub(crate) fn mailto(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v)
        .to_string()
}

pub(crate) fn partstat(s: Option<&str>) -> PartStat {
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
