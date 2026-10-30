//! Date-time values: UTC, zoned, floating and all-day; durations.

use super::*;

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

pub(crate) fn parse_time(p: &Prop) -> Option<IcalTime> {
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
pub(crate) fn normalize_tzid(tz: &str) -> String {
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

/// Longest DURATION we accept (about ten years); anything longer is hostile
/// or a mistake, and would overflow date arithmetic further on.
pub(crate) const MAX_DURATION_DAYS: i64 = 3653;

pub(crate) fn parse_duration(s: &str) -> Option<Duration> {
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
        let part = match c {
            '0'..='9' => {
                num.push(c);
                continue;
            }
            'T' => {
                in_time = true;
                continue;
            }
            'W' => Duration::try_weeks(num.parse().ok()?)?,
            'D' => Duration::try_days(num.parse().ok()?)?,
            'H' if in_time => Duration::try_hours(num.parse().ok()?)?,
            'M' if in_time => Duration::try_minutes(num.parse().ok()?)?,
            'S' if in_time => Duration::try_seconds(num.parse().ok()?)?,
            _ => return None,
        };
        total = total.checked_add(&part)?;
        if total > Duration::days(MAX_DURATION_DAYS) {
            return None;
        }
        num.clear();
    }
    Some(if neg { -total } else { total })
}

pub(crate) fn fmt_utc(d: &DateTime<Utc>) -> String {
    d.format("%Y%m%dT%H%M%SZ").to_string()
}

pub(crate) fn time_prop(name: &str, t: &DateTime<Utc>, all_day: bool, tz: Option<&str>) -> String {
    if all_day {
        return format!("{name};VALUE=DATE:{}", t.format("%Y%m%d"));
    }
    if let Some(z) = tz.and_then(|z| z.parse::<Tz>().ok())
        && z != chrono_tz::UTC {
            return format!("{name};TZID={}:{}", z.name(), t.with_timezone(&z).format("%Y%m%dT%H%M%S"));
        }
    format!("{name}:{}", fmt_utc(t))
}
