use super::*;

#[test]
fn schema_is_versioned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("satchel.db");
    drop(Store::open(&path).unwrap());
    let c = Connection::open(&path).unwrap();
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    // Reopening is a no-op; a database from a newer Satchel is refused.
    drop(Store::open(&path).unwrap());
    c.pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
    drop(c);
    assert!(Store::open(&path).err().unwrap().to_string().contains("newer version"));
}

#[test]
fn a_database_from_before_versioning_counts_as_version_1() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("satchel.db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch(include_str!("schema.sql")).unwrap();
    // Back to the version 1 shape, so the migrations since run on it.
    c.execute_batch("ALTER TABLE messages DROP COLUMN partial; ALTER TABLE messages DROP COLUMN blob_size;").unwrap();
    drop(c);
    drop(Store::open(&path).unwrap());
    let c = Connection::open(&path).unwrap();
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    c.query_row("SELECT partial, blob_size FROM messages", [], |_| Ok(())).optional().unwrap();
}

fn rec(remote: &str, mid: &str, irt: Option<&str>, subject: &str, mins: i64) -> MessageRecord {
    MessageRecord {
        remote_id: remote.into(),
        message_id_hdr: Some(mid.into()),
        in_reply_to: irt.map(|s| s.into()),
        references: irt.map(|s| vec![s.to_string()]).unwrap_or_default(),
        subject: subject.into(),
        from: Some(Address::new("bob@x.test")),
        date: Utc::now() + chrono::Duration::minutes(mins),
        snippet: subject.into(),
        ..Default::default()
    }
}

#[test]
fn threading_out_of_order() {
    let s = Store::open_in_memory().unwrap();
    let acc = s
        .insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() })
        .unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    // Child arrives first, then an unrelated message, then the parent.
    let (c, _) = s.upsert_message(&acc.id, &rec("2", "m2", Some("m1"), "Re: hi", 1), &[(mb.clone(), Some(2))]).unwrap();
    let (u, _) = s.upsert_message(&acc.id, &rec("3", "m3", None, "other", 2), &[(mb.clone(), Some(3))]).unwrap();
    let (p, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "hi", 0), &[(mb.clone(), Some(1))]).unwrap();
    let tc = s.thread_of(&c).unwrap().unwrap();
    assert_eq!(s.thread_of(&p).unwrap().unwrap(), tc);
    assert_ne!(s.thread_of(&u).unwrap().unwrap(), tc);
    let t = s.get_thread_summary(&tc).unwrap().unwrap();
    assert_eq!(t.message_count, 2);
    assert_eq!(t.unread_count, 2);
    assert_eq!(t.subject, "hi");

    let list = s.list_threads(&ThreadFilter { mailbox_id: Some(mb.clone()), limit: 10, ..Default::default() }).unwrap();
    assert_eq!(list.len(), 2);
    // Newest first: the unrelated message is the most recent.
    assert_eq!(list[0].thread_id, s.thread_of(&u).unwrap().unwrap());
    assert_eq!(list[1].thread_id, tc);
}

#[test]
fn renaming_a_folder_rekeys_only_its_messages() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "Old", "Old", "Old", MailboxRole::Custom, false).unwrap();
    let (a, _) = s.upsert_message(&acc.id, &rec("7:1:Old", "m1", None, "a", 0), &[(mb.clone(), Some(1))]).unwrap();
    let (b, _) = s.upsert_message(&acc.id, &rec("7:2:Parent/Old", "m2", None, "b", 0), &[(mb.clone(), Some(2))]).unwrap();
    assert_eq!(s.rekey_mailbox_messages(&acc.id, "Old", "New").unwrap(), 1);
    assert_eq!(s.message_ref(&a).unwrap().unwrap().remote_id, "7:1:New");
    assert_eq!(s.message_ref(&b).unwrap().unwrap().remote_id, "7:2:Parent/Old");
}

#[test]
fn paging_with_identical_timestamps() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let when = Utc::now();
    for i in 0..7 {
        let mut r = rec(&format!("r{i}"), &format!("m{i}"), None, &format!("s{i}"), 0);
        r.date = when;
        s.upsert_message(&acc.id, &r, &[(mb.clone(), None)]).unwrap();
    }
    // Keyset: page size 2 must still visit all 7 threads exactly once.
    let mut seen = vec![];
    let mut f = ThreadFilter { mailbox_id: Some(mb.clone()), limit: 2, ..Default::default() };
    loop {
        let page = s.list_threads(&f).unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().unwrap();
        f.before = Some(last.last_date);
        f.before_id = Some(last.thread_id.clone());
        seen.extend(page.into_iter().map(|t| t.thread_id));
    }
    let mut uniq = seen.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(seen.len(), 7);
    assert_eq!(uniq.len(), 7);
    // Offset paging agrees with the total order and the count.
    let all = s.list_threads(&ThreadFilter { mailbox_id: Some(mb.clone()), limit: 100, ..Default::default() }).unwrap();
    let third = s.list_threads(&ThreadFilter { mailbox_id: Some(mb.clone()), limit: 2, offset: 4, ..Default::default() }).unwrap();
    assert_eq!(third[0].thread_id, all[4].thread_id);
    assert_eq!(s.count_threads(&ThreadFilter { mailbox_id: Some(mb), ..Default::default() }).unwrap(), 7);
}

#[test]
fn ops_run_in_order_and_a_retrying_op_holds_back_later_ones() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let kinds = |s: &Store| s.due_ops(&acc.id).unwrap().into_iter().map(|o| o.kind).collect::<Vec<_>>();
    // Queued within the same millisecond: still in queue order.
    let mv = s.enqueue_op(&acc.id, "move", &serde_json::json!({}), "user").unwrap();
    let send = s.enqueue_op(&acc.id, "send", &serde_json::json!({}), "user").unwrap();
    s.enqueue_op(&acc.id, "mailbox_delete", &serde_json::json!({}), "user").unwrap();
    assert_eq!(kinds(&s), vec!["move", "send", "mailbox_delete"]);
    // A send waiting to retry doesn't hold anything back...
    s.op_retry(&send, "down", 1).unwrap();
    assert_eq!(kinds(&s), vec!["move", "mailbox_delete"]);
    // ...but any other op does, along with everything after it.
    s.op_retry(&mv, "down", 1).unwrap();
    assert!(kinds(&s).is_empty());
}

#[test]
fn unread_ids_by_mailbox_or_role() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let inbox = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let work = s.upsert_mailbox(&acc.id, "Work", "Work", "Work", MailboxRole::Custom, false).unwrap();
    let mut read = rec("3", "m3", None, "read", 2);
    read.flags.seen = true;
    let (a, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "a", 0), &[(inbox.clone(), None)]).unwrap();
    let (b, _) = s.upsert_message(&acc.id, &rec("2", "m2", None, "b", 1), &[(work.clone(), None)]).unwrap();
    s.upsert_message(&acc.id, &read, &[(inbox.clone(), None)]).unwrap();
    assert_eq!(s.unread_ids(Some(&inbox), None).unwrap(), vec![a]);
    assert_eq!(s.unread_ids(None, Some(MailboxRole::Custom)).unwrap(), vec![b]);
    assert!(s.unread_ids(None, Some(MailboxRole::Trash)).unwrap().is_empty());
}

#[test]
fn with_search_and_person_summary() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let mut from_bob = rec("1", "m1", None, "from bob", 0);
    from_bob.from = Some(Address { name: Some("Bob Smith".into()), email: "Bob@X.test".into() });
    from_bob.attachments = vec![Attachment { id: "1".into(), filename: "a.pdf".into(), mime_type: "application/pdf".into(), size: 1, inline: false, content_id: None, risky: false }];
    let mut to_bob = rec("2", "m2", None, "to bob at work", 1);
    to_bob.from = Some(Address::new("a@x.test"));
    to_bob.cc = vec![Address::new("bob@work.test")];
    // A look-alike address must not match exactly.
    let mut other = rec("3", "m3", None, "jimbob", 2);
    other.from = Some(Address::new("jimbob@x.test"));
    other.to = vec![Address::new("xbob@work.test")];
    for r in [&from_bob, &to_bob, &other] {
        s.upsert_message(&acc.id, r, &[(mb.clone(), None)]).unwrap();
    }
    let with = |v: &[&str]| s.count_threads(&ThreadFilter { with: v.iter().map(|x| x.to_string()).collect(), ..Default::default() }).unwrap();
    assert_eq!(with(&["bob@x.test"]), 1);
    assert_eq!(with(&["bob@x.test", "bob@work.test"]), 2);
    assert_eq!(with(&["bob"]), 3);
    assert_eq!(s.count_threads(&ThreadFilter { from: vec!["bob@x.test".into()], ..Default::default() }).unwrap(), 1);

    let p = s.person_summary(&["bob@x.test".into(), "BOB@work.test".into()]).unwrap();
    assert_eq!((p.received, p.sent, p.attachments), (1, 1, 1));
    assert_eq!(p.name.as_deref(), Some("Bob Smith"));
    assert_eq!(p.addresses[1].email, "bob@work.test");
    assert!(p.first_date.unwrap() < p.last_date.unwrap());
}
