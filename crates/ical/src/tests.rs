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

#[test]
fn hostile_durations_do_not_panic() {
    for d in ["P99999999999999W", "P99999999D", "PT9999999999999H", "PT99999999999999999999S", "P3654D"] {
        assert_eq!(parse_duration(d), None, "{d}");
    }
    assert_eq!(parse_duration("P520W"), Some(Duration::weeks(520)));
    assert_eq!(parse_duration("-PT15M"), Some(-Duration::minutes(15)));
    for d in ["P99999999999999W", "P99999999D", "PT9999999999999H"] {
        let ics = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260105T090000Z\r\nDURATION:{d}\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let c = parse(&ics);
        let e = &c.events[0];
        assert_eq!(e.end_utc(), e.start_utc());
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
        assert_eq!(expand(e, &[], from, to).len(), 3);
    }
    // An all-day event on the last representable date doesn't overflow either.
    let max = parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:y\r\nDTSTART;VALUE=DATE:+2621431231\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    if let Some(e) = max.events.first() {
        let _ = e.end_utc();
        let _ = expand(e, &[], DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC);
    }
}
