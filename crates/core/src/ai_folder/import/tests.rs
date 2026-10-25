use super::*;

#[test]
fn parses_scalars_lists_and_comments() {
    let (fm, body) = parse(
        "---\naction: archive\nthreads: [a1, \"b, 2\"]\nwhy: 'it''s done' # trailing comment\nto:\n  - Bob <bob@x.test>\n  - carol@x.test\nsubject: \"Re: \\\"hi\\\" # not a comment\"\n---\nBody text\n",
    )
    .unwrap();
    assert_eq!(fm.str("action"), Some("archive"));
    assert_eq!(fm.list("threads"), vec!["a1", "b, 2"]);
    assert_eq!(fm.str("why"), Some("it's done"));
    assert_eq!(fm.list("to"), vec!["Bob <bob@x.test>", "carol@x.test"]);
    assert_eq!(fm.str("subject"), Some("Re: \"hi\" # not a comment"));
    assert_eq!(body, "Body text\n");
}

#[test]
fn requires_front_matter() {
    assert!(parse("action: archive\n").is_err());
    assert!(parse("---\naction: archive\n").is_err());
}

#[test]
fn set_fields_replaces_and_removes_keys() {
    let text = "---\naction: reply\nagent_token: stk_secret\nwarnings:\n  - old\nwhy: ok\n---\nHello\n";
    let out = set_fields(text, &[("agent_token", None), ("warnings", None), ("status", Some("pending".into()))]);
    assert_eq!(out, "---\naction: reply\nwhy: ok\nstatus: pending\n---\nHello\n");
    let again = set_fields(&out, &[("status", Some("rejected".into())), ("note", Some(quote("not now\n\"sorry\"")))]);
    let (fm, body) = parse(&again).unwrap();
    assert_eq!(fm.str("status"), Some("rejected"));
    assert_eq!(fm.str("note"), Some("not now\n\"sorry\""));
    assert_eq!(body, "Hello\n");
}

#[test]
fn event_times_and_all_day_dates() {
    let (t, all_day) = when("start", "2026-11-02T10:00:00-05:00").unwrap();
    assert_eq!(t.to_rfc3339(), "2026-11-02T15:00:00+00:00");
    assert!(!all_day);
    assert!(when("start", "2026-11-02").unwrap().1);
    assert!(when("start", "next tuesday").is_err());
}

#[test]
fn parses_addresses() {
    assert_eq!(address("Bob Smith <bob@x.test>").unwrap(), Address { name: Some("Bob Smith".into()), email: "bob@x.test".into() });
    assert_eq!(address("bob@x.test").unwrap(), Address::new("bob@x.test"));
    assert!(address("bob").is_err());
}

#[test]
fn after_a_restart_only_current_person_files_count_as_edits() {
    let store = Store::open_in_memory().unwrap();
    let fields = ContactFields {
        name: "Bob".into(),
        emails: vec![ContactEmail { email: "bob@x.test".into(), label: Some("work".into()) }, ContactEmail { email: "old@x.test".into(), label: Some("home".into()) }],
        ..Default::default()
    };
    let id = store.insert_contact(&fields, ContactOrigin::Local).unwrap();
    let c = store.get_contact(&id).unwrap().unwrap();
    let file = crate::ai_folder::contact_file(&Contact { id: Some(id.clone()), fields: c.fields.clone(), version: c.version, ..Default::default() }, &Default::default());
    assert!(read_person(&store, &file, None).unwrap().is_none(), "unchanged");

    // Removing a labeled address keeps the other's label.
    let edited = file.replace("emails: [\"bob@x.test\",\"old@x.test\"]", "emails: [\"bob@x.test\"]");
    let d = read_person(&store, &edited, None).unwrap().expect("an edit");
    let ProposalAction::SaveContact { contact } = d.action else { panic!() };
    assert_eq!(contact.fields.emails, vec![ContactEmail { email: "bob@x.test".into(), label: Some("work".into()) }]);

    // The contact moved on since: an unremembered file is just out of date.
    let mut f = c.fields.clone();
    f.title = "Chef".into();
    store.update_contact(&id, &f, None).unwrap();
    assert!(read_person(&store, &edited, None).unwrap().is_none());
    // Remembered, the same edit applies on top of the newer contact.
    let d = read_person(&store, &edited, Some(&file)).unwrap().expect("an edit");
    let ProposalAction::SaveContact { contact } = d.action else { panic!() };
    assert_eq!((contact.fields.emails.len(), contact.fields.title.as_str()), (1, "Chef"));
}
