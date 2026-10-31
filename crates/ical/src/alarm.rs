//! VALARMs: reading an event's alerts and writing them back (0056).

use super::*;

/// Marks an alarm that came from a calendar's default reminders, so a
/// provider can tell "still the defaults" from an explicit choice.
pub const DEFAULT_MARK: &str = "X-SATCHEL-DEFAULT:TRUE";

/// When an alarm goes off.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Offset from the start (negative: before).
    Start(Duration),
    /// Offset from the end.
    End(Duration),
    At(DateTime<Utc>),
}

#[derive(Debug, Clone)]
pub struct ParsedAlarm {
    /// DISPLAY, AUDIO, EMAIL… (uppercase).
    pub action: String,
    pub trigger: Option<Trigger>,
    /// From a calendar's default reminders (see [`DEFAULT_MARK`]).
    pub default: bool,
}

impl ParsedAlarm {
    /// Alarms that show on screen (or play a sound): the ones Satchel fires.
    pub fn shows(&self) -> bool {
        matches!(self.action.as_str(), "DISPLAY" | "AUDIO" | "")
    }

    pub(crate) fn read(&mut self, p: &Prop) {
        match p.name.as_str() {
            "ACTION" => self.action = p.value.trim().to_ascii_uppercase(),
            "TRIGGER" => {
                let v = p.value.trim();
                self.trigger = if p.param("VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE-TIME")) {
                    parse_time(p).map(|t| Trigger::At(t.to_utc()))
                } else {
                    let d = parse_duration(v);
                    match p.param("RELATED") {
                        Some(r) if r.eq_ignore_ascii_case("END") => d.map(Trigger::End),
                        _ => d.map(Trigger::Start),
                    }
                };
            }
            _ if format!("{}:{}", p.name, p.value.trim()).eq_ignore_ascii_case(DEFAULT_MARK) => self.default = true,
            _ => {}
        }
    }
}

impl ParsedEvent {
    /// The alarms that show, as minutes before the start.
    pub fn alerts(&self) -> Vec<Alert> {
        self.alarms.iter().filter(|a| a.shows()).filter_map(|a| Some(Alert { minutes_before: self.minutes_before(a)? })).collect()
    }

    /// When `a` goes off, in minutes before this event's start.
    pub fn minutes_before(&self, a: &ParsedAlarm) -> Option<i32> {
        let before = match a.trigger.as_ref()? {
            Trigger::Start(d) => -*d,
            Trigger::End(d) => -(self.duration() + *d),
            Trigger::At(t) => self.start_utc() - *t,
        };
        i32::try_from(before.num_minutes()).ok()
    }
}

/// A VALARM block, folded and CRLF-separated, without a trailing CRLF.
pub fn valarm(minutes_before: i32, action: &str, extra: &[&str]) -> String {
    let m = i64::from(minutes_before);
    let trigger = if m == 0 { "PT0S".to_string() } else { format!("{}PT{}M", if m > 0 { "-" } else { "" }, m.abs()) };
    let mut lines = vec!["BEGIN:VALARM".to_string(), format!("ACTION:{action}"), format!("TRIGGER:{trigger}")];
    if action != "AUDIO" {
        lines.push("DESCRIPTION:Reminder".into());
    }
    if action == "EMAIL" {
        lines.push("SUMMARY:Reminder".into());
    }
    lines.extend(extra.iter().map(|s| s.to_string()));
    lines.push("END:VALARM".into());
    lines.iter().map(|l| fold(l)).collect::<String>().trim_end().to_string()
}

/// The VALARM blocks of the master component (the VEVENT without a
/// RECURRENCE-ID), refolded, each with whether it shows on screen.
pub fn alarm_blocks(ics: &str) -> Vec<(String, bool)> {
    let mut found: Option<Vec<(String, bool)>> = None;
    let mut cur: Vec<(String, bool)> = vec![];
    let mut alarm: Option<(String, ParsedAlarm)> = None;
    let (mut in_event, mut is_override, mut depth) = (false, false, 0);
    for l in unfold(ics) {
        let upper = l.to_ascii_uppercase();
        if upper == "BEGIN:VEVENT" {
            (in_event, is_override, depth) = (true, false, 0);
            cur.clear();
            continue;
        }
        if !in_event {
            continue;
        }
        if upper == "END:VEVENT" {
            in_event = false;
            if !is_override && found.is_none() {
                found = Some(std::mem::take(&mut cur));
            }
            continue;
        }
        if upper.starts_with("BEGIN:") {
            depth += 1;
            if depth == 1 && upper == "BEGIN:VALARM" {
                alarm = Some((String::new(), ParsedAlarm { action: String::new(), trigger: None, default: false }));
            }
        }
        if let Some((text, a)) = alarm.as_mut() {
            text.push_str(&fold(&l));
            if depth == 1 && let Some(p) = parse_line(&l) {
                a.read(&p);
            }
        }
        if upper.starts_with("END:") {
            depth -= 1;
            if depth == 0 && let Some((text, a)) = alarm.take() {
                cur.push((text.trim_end().to_string(), a.shows()));
            }
        } else if depth == 0 && upper.starts_with("RECURRENCE-ID") {
            is_override = true;
        }
    }
    found.unwrap_or_default()
}
