//! Calendar alerts (0056): edits keep alarms set elsewhere, and alerts go
//! off once, snooze and dismiss.

mod support;

use chrono::{DateTime, Duration, Utc};
use support::mail::*;

/// An event synced from the server with an alarm another client set (with
/// a property Satchel doesn't model) and an email alarm.
fn ics(start: DateTime<Utc>) -> String {
    let t = |d: DateTime<Utc>| d.format("%Y%m%dT%H%M%SZ").to_string();
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:review-1\r\nDTSTAMP:20261001T000000Z\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Review\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Review\r\nX-WR-ALARMUID:abc-123\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-P1D\r\nDESCRIPTION:Review\r\nSUMMARY:Review\r\nATTENDEE:mailto:me@x.test\r\nEND:VALARM\r\n\
         END:VEVENT\r\nEND:VCALENDAR\r\n",
        t(start),
        t(start + Duration::hours(1))
    )
}

/// A whole second `n` days from now (iCalendar has no fractions).
fn in_days(n: i64) -> DateTime<Utc> {
    DateTime::from_timestamp((Utc::now() + Duration::days(n)).timestamp(), 0).unwrap()
}

async fn synced(start: DateTime<Utc>) -> (std::sync::Arc<Engine>, String, tempfile::TempDir) {
    let (engine, fake, account, dir) = setup().await;
    fake.events.lock().unwrap().push(("review".into(), ics(start)));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| !engine.store().events_by_uid(&account, "review-1").unwrap().is_empty()).await;
    let id = engine.store().events_by_uid(&account, "review-1").unwrap().remove(0).id;
    (engine, id, dir)
}

async fn get(engine: &std::sync::Arc<Engine>, id: &str) -> Event {
    serde_json::from_value(engine.handle(&Caller::User, Command::GetEvent { event_id: id.into() }).await.unwrap()).unwrap()
}

async fn update(engine: &std::sync::Arc<Engine>, e: Event) {
    engine.handle(&Caller::User, Command::UpdateEvent { event: e }).await.unwrap();
}

fn stored_ics(engine: &Engine, id: &str) -> String {
    engine.store().get_event(id).unwrap().unwrap().ics
}

#[tokio::test(flavor = "multi_thread")]
async fn edits_keep_alarms_set_elsewhere() {
    let (engine, id, _dir) = synced(in_days(3)).await;
    let mut e = get(&engine, &id).await;
    assert_eq!(e.alerts, Some(vec![Alert { minutes_before: 15 }]), "the email alarm isn't one that shows");

    // A title change, by a client that knows about alerts and by one that doesn't.
    e.title = "Design review".into();
    update(&engine, e.clone()).await;
    let mut old_client = e.clone();
    old_client.alerts = None;
    old_client.title = "Design review 2".into();
    update(&engine, old_client).await;
    let ics = stored_ics(&engine, &id);
    assert!(ics.contains("X-WR-ALARMUID:abc-123") && ics.contains("ACTION:EMAIL"), "{ics}");
    assert_eq!(ics.matches("BEGIN:VALARM").count(), 2, "{ics}");

    // Changing the alerts replaces the ones that show; the email one stays.
    e.alerts = Some(vec![Alert { minutes_before: 5 }, Alert { minutes_before: 60 }]);
    update(&engine, e).await;
    let ics = stored_ics(&engine, &id);
    assert!(!ics.contains("abc-123") && ics.contains("ACTION:EMAIL"), "{ics}");
    assert_eq!(get(&engine, &id).await.alerts, Some(vec![Alert { minutes_before: 5 }, Alert { minutes_before: 60 }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn alerts_go_off_once_and_snooze_and_dismiss() {
    let start = in_days(3);
    let (engine, id, _dir) = synced(start).await;
    let alerts = |engine: &std::sync::Arc<Engine>| {
        let engine = engine.clone();
        async move { serde_json::from_value::<Vec<CalendarAlert>>(engine.handle(&Caller::User, Command::ListAlerts).await.unwrap()).unwrap() }
    };

    // Not yet; then once, with the next one 15 minutes before the start.
    let (due, next) = engine.alerts_pass(start - Duration::minutes(20)).unwrap();
    assert!(due.is_empty());
    assert_eq!(next, Some(start - Duration::minutes(15)));
    let (due, _) = engine.alerts_pass(start - Duration::minutes(14)).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].event_id.as_str(), due[0].title.as_str(), due[0].minutes_before), (id.as_str(), "Review", 15));
    assert!(engine.alerts_pass(start - Duration::minutes(13)).unwrap().0.is_empty(), "it doesn't go off twice");
    assert_eq!(alerts(&engine).await.len(), 1);

    // Snoozing hides it until then (the pass's clock is the test's, so it
    // snoozes to a real future time and checks after it).
    let alert = due[0].id.clone();
    let until = Utc::now() + Duration::minutes(5);
    engine.handle(&Caller::User, Command::SnoozeAlert { alert_id: alert.clone(), until }).await.unwrap();
    assert!(alerts(&engine).await[0].snoozed);
    assert!(engine.handle(&Caller::User, Command::SnoozeAlert { alert_id: alert.clone(), until: Utc::now() - Duration::minutes(1) }).await.is_err());
    let (due, _) = engine.alerts_pass(start - Duration::minutes(12)).unwrap();
    assert_eq!(due.len(), 1, "the snooze ended (it's before the pass's clock)");
    assert!(!alerts(&engine).await[0].snoozed);

    engine.handle(&Caller::User, Command::DismissAlert { alert_id: None }).await.unwrap();
    assert!(alerts(&engine).await.is_empty());
    assert!(engine.alerts_pass(start - Duration::minutes(5)).unwrap().0.is_empty(), "dismissed for good");

    // Alerts are the user's: agents can't list or act on them.
    let agent = agent(&engine, AgentPolicy::default()).await;
    assert!(engine.handle(&agent, Command::ListAlerts).await.is_err());
    assert!(engine.handle(&agent, Command::DismissAlert { alert_id: None }).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn late_moved_and_turned_off_alerts() {
    let start = in_days(3);
    let (engine, id, _dir) = synced(start).await;

    // Over an hour late (nothing was running): dropped, not shown.
    assert!(engine.alerts_pass(start + Duration::minutes(50)).unwrap().0.is_empty());

    // An alert that went off stops showing when its event moves.
    let (due, _) = engine.alerts_pass(start - Duration::minutes(10)).unwrap();
    assert_eq!(due.len(), 1);
    let mut e = get(&engine, &id).await;
    e.start = start + Duration::days(1);
    e.end = e.start + Duration::hours(1);
    update(&engine, e.clone()).await;
    engine.alerts_pass(start - Duration::minutes(9)).unwrap();
    assert!(serde_json::from_value::<Vec<CalendarAlert>>(engine.handle(&Caller::User, Command::ListAlerts).await.unwrap()).unwrap().is_empty());

    // Turned off in settings: nothing goes off.
    let mut s = engine.settings();
    s.calendar_alerts = false;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    assert!(engine.alerts_pass(e.start - Duration::minutes(10)).unwrap().0.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn new_events_get_the_default_alert() {
    let (engine, _fake, _account, _dir) = setup().await;
    let cal = engine.store().list_calendars(None).unwrap().into_iter().find(|c| !c.cal.read_only).unwrap().cal.id;
    let start = Utc::now() + Duration::days(2);
    let event = |all_day: bool, alerts: Option<Vec<Alert>>| Event {
        id: None,
        calendar_id: cal.clone(),
        uid: None,
        title: "Lunch".into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + Duration::hours(1),
        all_day,
        timezone: None,
        rrule: None,
        attendees: vec![],
        organizer: None,
        status: EventStatus::Confirmed,
        my_status: None,
        alerts,
    };
    let create = |e: Event| {
        let engine = engine.clone();
        async move {
            let e: Event = serde_json::from_value(engine.handle(&Caller::User, Command::CreateEvent { event: e }).await.unwrap()).unwrap();
            get(&engine, e.id.as_deref().unwrap()).await.alerts
        }
    };
    assert_eq!(create(event(false, None)).await, Some(vec![Alert { minutes_before: 10 }]));
    assert_eq!(create(event(true, None)).await, Some(vec![]), "all-day events have no default");
    assert_eq!(create(event(false, Some(vec![]))).await, Some(vec![]), "an explicit none stays none");
}

#[tokio::test(flavor = "multi_thread")]
async fn upcoming_alerts_can_be_acted_on_before_they_go_off() {
    let start = in_days(3);
    let (engine, id, _dir) = synced(start).await;
    let upcoming = |until: DateTime<Utc>| {
        let engine = engine.clone();
        async move { serde_json::from_value::<Vec<CalendarAlert>>(engine.handle(&Caller::User, Command::ListUpcomingAlerts { until }).await.unwrap()).unwrap() }
    };
    assert!(upcoming(start - Duration::days(1)).await.is_empty(), "not by then");
    let list = upcoming(start).await;
    assert_eq!(list.len(), 1);
    let alert = list[0].clone();
    assert_eq!(alert.due, start - Duration::minutes(15));
    assert_eq!(satchel_core::calendar::parse_alert_id(&alert.id), Some((id.clone(), start, 15)));

    // The OS showed it while the app was closed, and the user snoozed it there.
    let until = Utc::now() + Duration::hours(2);
    engine.handle(&Caller::User, Command::SnoozeAlert { alert_id: alert.id.clone(), until }).await.unwrap();
    let list = upcoming(Utc::now() + Duration::days(4)).await;
    assert_eq!(list.len(), 1);
    assert!(list[0].snoozed && list[0].due.timestamp() == until.timestamp(), "{list:?}");
    // Then dismissed it: it never goes off here.
    engine.handle(&Caller::User, Command::DismissAlert { alert_id: Some(alert.id.clone()) }).await.unwrap();
    assert!(upcoming(Utc::now() + Duration::days(4)).await.is_empty());
    assert!(engine.alerts_pass(start - Duration::minutes(14)).unwrap().0.is_empty());

    // An id that isn't an alert of a real event is refused.
    assert!(engine.handle(&Caller::User, Command::DismissAlert { alert_id: Some(format!("{id}:0:15")) }).await.is_err());
    assert!(engine.handle(&Caller::User, Command::DismissAlert { alert_id: Some("nonsense".into()) }).await.is_err());
}
