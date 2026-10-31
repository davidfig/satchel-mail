//! Calendar writes: moving events between calendars.

mod support;

use support::mail::*;

fn calendar(engine: &Engine, name: &str) -> String {
    engine.store().list_calendars(None).unwrap().into_iter().find(|c| c.cal.name == name).unwrap().cal.id
}

const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:dentist-1\r\nDTSTAMP:20261001T000000Z\r\nDTSTART:20261105T160000Z\r\nDTEND:20261105T170000Z\r\nSUMMARY:Dentist\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[tokio::test(flavor = "multi_thread")]
async fn moving_an_event_keeps_its_uid_and_deletes_the_old_copy_silently() {
    let (engine, fake, account, _dir) = setup().await;
    fake.events.lock().unwrap().push(("dentist".into(), ICS.into()));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| !engine.store().events_by_uid(&account, "dentist-1").unwrap().is_empty()).await;
    let old = engine.store().events_by_uid(&account, "dentist-1").unwrap().remove(0);
    let mut e: Event = serde_json::from_value(engine.handle(&Caller::User, Command::GetEvent { event_id: old.id.clone() }).await.unwrap()).unwrap();

    let personal = calendar(&engine, "Personal");
    e.calendar_id = personal.clone();
    let moved: Event = serde_json::from_value(engine.handle(&Caller::User, Command::UpdateEvent { event: e }).await.unwrap()).unwrap();

    let now = engine.store().events_by_uid(&account, "dentist-1").unwrap();
    assert_eq!(now.len(), 1, "one copy, in the new calendar");
    assert_eq!(now[0].calendar_id, personal);
    assert_eq!(moved.id.as_deref(), Some(now[0].id.as_str()));
    wait_for(|| !fake.cal_deletes.lock().unwrap().is_empty()).await;
    assert_eq!(*fake.cal_deletes.lock().unwrap(), vec![false], "the old copy goes without notifying anyone");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_with_guests_stay_in_their_calendar() {
    let (engine, _fake, account, _dir) = setup().await;
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let new = Event {
        id: None,
        calendar_id: calendar(&engine, "Calendar"),
        uid: None,
        title: "Planning".into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        timezone: None,
        rrule: None,
        attendees: vec![Attendee { email: "alice@x.test".into(), name: None, status: PartStat::NeedsAction, optional: false }],
        organizer: None,
        status: EventStatus::Confirmed,
        my_status: None,
        alerts: None,
    };
    let mut e: Event = serde_json::from_value(engine.handle(&Caller::User, Command::CreateEvent { event: new }).await.unwrap()).unwrap();
    let home = e.calendar_id.clone();

    // Dropping the guests in the same edit doesn't get around it: they were invited.
    e.calendar_id = calendar(&engine, "Personal");
    e.attendees.clear();
    let err = engine.handle(&Caller::User, Command::UpdateEvent { event: e.clone() }).await.unwrap_err();
    assert!(err.to_string().contains("guests"), "{err}");
    let uid = e.uid.clone().unwrap();
    let stored = engine.store().events_by_uid(&account, &uid).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].calendar_id, home);
}

#[tokio::test(flavor = "multi_thread")]
async fn locations_come_from_history_and_only_go_online_when_allowed() {
    let (engine, fake, _account, _dir) = setup().await;
    let cal = calendar(&engine, "Calendar");
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    for (title, location) in [("Standup", "Room 4"), ("Retro", "Room 4"), ("Lunch", "Rose Cafe, 5 Main St"), ("Call", "")] {
        let e = Event {
            id: None,
            calendar_id: cal.clone(),
            uid: None,
            title: title.into(),
            description: String::new(),
            location: location.into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            timezone: None,
            rrule: None,
            attendees: vec![],
            organizer: None,
            status: EventStatus::Confirmed,
            my_status: None,
            alerts: None,
        };
        engine.handle(&Caller::User, Command::CreateEvent { event: e }).await.unwrap();
    }
    let suggest = |q: &str| {
        let engine = engine.clone();
        let q = q.to_string();
        async move { serde_json::from_value::<Vec<PlaceSuggestion>>(engine.handle(&Caller::User, Command::SuggestLocations { query: q }).await.unwrap()).unwrap() }
    };

    // Most used first; nothing leaves the device while the setting is off.
    let all = suggest("").await;
    assert_eq!(all.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Room 4", "Rose Cafe, 5 Main St"]);
    assert_eq!(suggest("ro").await.len(), 2);
    assert!(suggest("rose").await.iter().all(|s| s.source == PlaceSource::History));
    assert!(fake.place_queries.lock().unwrap().is_empty());

    let mut s = engine.settings();
    s.look_up_places = true;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    // Too short to look up; then history first, the service after.
    suggest("ro").await;
    assert!(fake.place_queries.lock().unwrap().is_empty());
    let found = suggest("rose").await;
    assert_eq!(found[0].source, PlaceSource::History);
    assert_eq!(found[1].text, "rose Cafe, 1 Main St");
    // Asking again uses the cache.
    suggest("Rose").await;
    assert_eq!(*fake.place_queries.lock().unwrap(), vec!["rose".to_string()]);

    // Agents can't use it.
    let a = agent(&engine, AgentPolicy::default()).await;
    assert!(engine.handle(&a, Command::SuggestLocations { query: "rose".into() }).await.is_err());
}
