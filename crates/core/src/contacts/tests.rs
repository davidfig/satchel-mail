use super::*;

fn e(s: &str) -> ContactEmail {
    ContactEmail { email: s.into(), label: None }
}

#[test]
fn machine_addresses() {
    for m in ["no-reply@x.test", "noreply@x.test", "do_not_reply@x.test", "MAILER-DAEMON@x.test", "notifications@github.com", "reply+AAAAAAAAAAAAAAAAAAAA@reply.github.com", "bounces+123@x.test", "news+abcdefghijklmnopqrstu@x.test", "hello@noreply.x.test", "nope"] {
        assert!(is_machine_address(m), "{m}");
    }
    for p in ["bob@x.test", "bob+work@x.test", "info@smallshop.test", "anne-marie@x.test"] {
        assert!(!is_machine_address(p), "{p}");
    }
}

#[test]
fn correspondents_skip_own_machines_and_spam_and_prefer_self_given_names() {
    let ev = vec![
        ("bob@x.test".to_string(), Some("bobby".to_string()), 10, false),
        ("bob@x.test".to_string(), Some("Bob Smith".to_string()), 20, true),
        ("BOB@x.test".to_string(), Some("\"bob@x.test\"".to_string()), 30, false),
        ("me@x.test".to_string(), Some("Me".to_string()), 5, false),
        ("noreply@x.test".to_string(), None, 5, true),
        ("spammer@x.test".to_string(), Some("Spam".to_string()), 5, true),
        ("carol@x.test".to_string(), None, 7, false),
    ];
    let own: HashSet<String> = ["me@x.test".to_string()].into();
    let people = correspondents(&ev, &own, |e| e == "spammer@x.test");
    assert_eq!(people.len(), 2);
    assert_eq!(people[0].email, "bob@x.test");
    assert_eq!(people[0].name.as_deref(), Some("Bob Smith"));
    assert_eq!(people[0].messages, 3);
    assert_eq!(people[0].last_date.timestamp_millis(), 30);
    assert_eq!(people[1].email, "carol@x.test");
    assert_eq!(people[1].name, None);
}

#[test]
fn three_way_merge_keeps_local_edits_and_takes_server_ones() {
    let base = ContactFields { name: "Bob".into(), emails: vec![e("bob@x.test"), e("old@x.test")], title: "Dev".into(), notes: "n".into(), ..Default::default() };
    // We renamed and added an address.
    let ours = ContactFields { name: "Robert".into(), emails: vec![e("bob@x.test"), e("old@x.test"), e("rob@y.test")], ..base.clone() };
    // They changed the title, removed old@, added an address, and relabeled bob@.
    let theirs = ContactFields {
        name: "Bob".into(),
        emails: vec![ContactEmail { email: "bob@x.test".into(), label: Some("work".into()) }, e("b@z.test")],
        title: "Lead".into(),
        ..base.clone()
    };
    let m = merge_fields(&base, &ours, &theirs);
    assert_eq!(m.name, "Robert");
    assert_eq!(m.title, "Lead");
    assert_eq!(m.notes, "n");
    let emails: Vec<_> = m.emails.iter().map(|x| (x.email.as_str(), x.label.as_deref())).collect();
    assert_eq!(emails, vec![("bob@x.test", Some("work")), ("rob@y.test", None), ("b@z.test", None)]);
    // Both changed the same field: ours wins.
    let ours2 = ContactFields { title: "CTO".into(), ..base.clone() };
    assert_eq!(merge_fields(&base, &ours2, &theirs).title, "CTO");
}

#[test]
fn union_pools_addresses_and_keeps_primary_values() {
    let a = ContactFields { name: "Bob Smith".into(), emails: vec![e("bob@x.test")], phones: vec![ContactPhone { number: "+1 (555) 0100".into(), label: None }], notes: "A".into(), ..Default::default() };
    let b = ContactFields {
        name: "Bobby".into(),
        emails: vec![e("BOB@x.test"), e("bob@home.test")],
        phones: vec![ContactPhone { number: "+15550100".into(), label: Some("cell".into()) }],
        organization: "Acme".into(),
        notes: "B".into(),
        ..Default::default()
    };
    let u = union_fields(&a, &b);
    assert_eq!(u.name, "Bob Smith");
    assert_eq!(u.emails.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), vec!["bob@x.test", "bob@home.test"]);
    assert_eq!(u.phones.len(), 1);
    assert_eq!(u.organization, "Acme");
    assert_eq!(u.notes, "A\n\nB");
}

#[test]
fn normalize_validates() {
    let f = ContactFields { name: "  Ann   Lee ".into(), emails: vec![e(" ann@x.test "), e("ANN@x.test"), e("")], birthday: Some("19900115".into()), ..Default::default() };
    let n = normalize(&f).unwrap();
    assert_eq!(n.name, "Ann Lee");
    assert_eq!(n.emails, vec![e("ann@x.test")]);
    assert_eq!(n.birthday.as_deref(), Some("1990-01-15"));
    assert!(normalize(&ContactFields { emails: vec![e("not an address")], ..Default::default() }).is_err());
    assert!(normalize(&ContactFields::default()).is_err());
    assert!(normalize(&ContactFields { name: "X".into(), birthday: Some("someday".into()), ..Default::default() }).is_err());
}
