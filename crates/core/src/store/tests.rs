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
    c.execute_batch("ALTER TABLE messages DROP COLUMN partial; ALTER TABLE messages DROP COLUMN blob_size; ALTER TABLE proposals DROP COLUMN processed_file; ALTER TABLE messages DROP COLUMN auth_serv; ALTER TABLE events DROP COLUMN location;").unwrap();
    // An event from then gets its location read out of its iCalendar.
    c.pragma_update(None, "foreign_keys", false).unwrap();
    let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nDTSTART:20261105T160000Z\r\nSUMMARY:Lunch\r\nLOCATION:Rose Cafe\\, Main St\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    c.execute("INSERT INTO events (id, calendar_id, account_id, remote_id, uid, ics, title, range_start, range_end) VALUES ('e1', 'c', 'a', 'r', 'u1', ?, 'Lunch', 0, 0)", [ics]).unwrap();
    drop(c);
    drop(Store::open(&path).unwrap());
    let c = Connection::open(&path).unwrap();
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, SCHEMA_VERSION);
    c.query_row("SELECT partial, blob_size FROM messages", [], |_| Ok(())).optional().unwrap();
    c.query_row("SELECT processed_file FROM proposals", [], |_| Ok(())).optional().unwrap();
    c.query_row("SELECT auth_serv FROM messages", [], |_| Ok(())).optional().unwrap();
    let location: String = c.query_row("SELECT location FROM events WHERE id='e1'", [], |r| r.get(0)).unwrap();
    assert_eq!(location, "Rose Cafe, Main St");
}

#[test]
fn background_reads_skip_the_users_connection() {
    let s = Store::open_in_memory().unwrap();
    // A slow user read holds its connection; background reads and the
    // data-version poll still go ahead.
    let _held = s.read.lock().unwrap();
    background(|| s.list_accounts()).unwrap();
    s.data_version().unwrap();
    // The flag ends with the closure.
    assert!(!BACKGROUND.with(Cell::get));
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
fn favorite_senders_mark_and_filter_threads() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let (from_bob, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "hi", 0), &[(mb.clone(), Some(1))]).unwrap();
    let mut r = rec("2", "m2", None, "other", 1);
    r.from = Some(Address::new("carol@x.test"));
    s.upsert_message(&acc.id, &r, &[(mb.clone(), Some(2))]).unwrap();
    let bob = s.thread_of(&from_bob).unwrap().unwrap();

    // Stored lowercase, so the address's case doesn't matter.
    assert_eq!(s.set_favorite_senders(&["Bob@X.test".into()], true).unwrap(), 1);
    assert_eq!(s.set_favorite_senders(&["bob@x.test".into()], true).unwrap(), 0);
    assert_eq!(s.list_favorite_senders().unwrap(), vec!["bob@x.test".to_string()]);
    let all = s.list_threads(&ThreadFilter { mailbox_id: Some(mb.clone()), limit: 10, ..Default::default() }).unwrap();
    assert_eq!(all.iter().filter(|t| t.favorite).map(|t| t.thread_id.clone()).collect::<Vec<_>>(), vec![bob.clone()]);
    assert!(s.get_thread_summary(&bob).unwrap().unwrap().favorite);
    let fav = ThreadFilter { favorite: true, limit: 10, ..Default::default() };
    assert_eq!(s.list_threads(&fav).unwrap().len(), 1);
    assert_eq!(s.count_threads(&fav).unwrap(), 1);

    s.set_favorite_senders(&["bob@x.test".into()], false).unwrap();
    assert!(s.list_favorite_senders().unwrap().is_empty());
    assert_eq!(s.count_threads(&fav).unwrap(), 0);
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
    let sent = s.upsert_mailbox(&acc.id, "Sent", "Sent", "Sent", MailboxRole::Sent, false).unwrap();
    for (r, m) in [(&from_bob, &mb), (&to_bob, &sent), (&other, &mb)] {
        s.upsert_message(&acc.id, r, &[(m.clone(), None)]).unwrap();
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

#[test]
fn a_huge_references_header_is_cut_down_and_still_threads() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let (parent, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "hi", 0), &[(mb.clone(), Some(1))]).unwrap();
    // 40,000 ids (over SQLite's bound-variable limit), the parent last.
    let mut r = rec("2", "m2", Some("m1"), "Re: hi", 1);
    r.references = (0..40_000).map(|i| format!("junk{i}@x.test")).chain(["m1".to_string()]).collect();
    let (child, _) = s.upsert_message(&acc.id, &r, &[(mb.clone(), Some(2))]).unwrap();
    assert_eq!(s.thread_of(&child).unwrap(), s.thread_of(&parent).unwrap());
    let stored = s.message_row(&child).unwrap().unwrap().references;
    assert_eq!(stored.len(), messages::MAX_REFERENCES);
    assert_eq!(stored.last().map(String::as_str), Some("m1"));
    let refs: i64 = s.r(|c| Ok(c.query_row("SELECT COUNT(*) FROM message_refs WHERE message_id=?", [&child], |r| r.get(0))?)).unwrap();
    assert_eq!(refs, messages::MAX_REFERENCES as i64);
}

#[test]
fn a_message_naming_two_threads_joins_one_and_merges_neither() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let (a, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "bank", 0), &[(mb.clone(), Some(1))]).unwrap();
    let (b, _) = s.upsert_message(&acc.id, &rec("2", "m2", None, "boss", 1), &[(mb.clone(), Some(2))]).unwrap();
    let mut r = rec("3", "m3", Some("m1"), "Re: both", 2);
    r.references = vec!["m1".into(), "m2".into()];
    let (c, _) = s.upsert_message(&acc.id, &r, &[(mb.clone(), Some(3))]).unwrap();
    let (ta, tb) = (s.thread_of(&a).unwrap().unwrap(), s.thread_of(&b).unwrap().unwrap());
    assert_ne!(ta, tb);
    // The most recent of the messages it names.
    assert_eq!(s.thread_of(&c).unwrap().unwrap(), tb);
    assert_eq!(s.get_thread_summary(&ta).unwrap().unwrap().message_count, 1);

    // Replies that arrived before their parent are still brought together.
    let (r1, _) = s.upsert_message(&acc.id, &rec("4", "m5", Some("m4"), "Re: plan", 4), &[(mb.clone(), Some(4))]).unwrap();
    let (r2, _) = s.upsert_message(&acc.id, &rec("5", "m6", Some("m4"), "Re: plan", 5), &[(mb.clone(), Some(5))]).unwrap();
    let (p, _) = s.upsert_message(&acc.id, &rec("6", "m4", None, "plan", 3), &[(mb.clone(), Some(6))]).unwrap();
    let t = s.thread_of(&p).unwrap();
    assert!(t == s.thread_of(&r1).unwrap() && t == s.thread_of(&r2).unwrap());
}

#[test]
fn a_reused_message_id_from_someone_else_is_not_a_copy() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let inbox = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let trash = s.upsert_mailbox(&acc.id, "Trash", "Trash", "Trash", MailboxRole::Trash, false).unwrap();
    let (real, _) = s.upsert_message(&acc.id, &rec("1", "m1", None, "invoice", 0), &[(inbox.clone(), Some(1))]).unwrap();
    // The server's own copy elsewhere.
    let (copy, _) = s.upsert_message(&acc.id, &rec("2", "m1", None, "invoice", 0), &[(trash, Some(1))]).unwrap();
    // A forgery reusing the Message-ID, from another sender or with other text.
    let mut forged = rec("3", "m1", None, "invoice", 1);
    forged.from = Some(Address::new("mallory@evil.test"));
    let (f1, _) = s.upsert_message(&acc.id, &forged, &[(inbox.clone(), Some(2))]).unwrap();
    let mut forged = rec("4", "m1", None, "pay here instead", 1);
    forged.from = Some(Address::new("BOB@x.test"));
    let (f2, _) = s.upsert_message(&acc.id, &forged, &[(inbox, Some(3))]).unwrap();
    assert_eq!(s.message_copies(&real).unwrap(), vec![copy.clone()]);
    assert!(s.message_copies(&f1).unwrap().is_empty() && s.message_copies(&f2).unwrap().is_empty());
    let t = s.get_thread_summary(&s.thread_of(&real).unwrap().unwrap()).unwrap().unwrap();
    assert_eq!(t.message_count, 3, "the real message once, and each forgery");
}

#[test]
fn only_sent_folder_mail_makes_correspondents() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "me@x.test".into(), display_name: "Me".into() }).unwrap();
    let sent = s.upsert_mailbox(&acc.id, "Sent", "Sent", "Sent", MailboxRole::Sent, false).unwrap();
    let junk = s.upsert_mailbox(&acc.id, "Junk", "Junk", "Junk", MailboxRole::Junk, false).unwrap();
    let inbox = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let mine = |rid: &str, to: &str| {
        let mut r = rec(rid, rid, None, "hello", 0);
        r.from = Some(Address::new("Me@x.test"));
        r.to = vec![Address::new(to)];
        r
    };
    s.upsert_message(&acc.id, &mine("1", "bob@x.test"), &[(sent, Some(1))]).unwrap();
    // Forged From: me, landing in Junk or the inbox.
    s.upsert_message(&acc.id, &mine("2", "mallory@evil.test"), &[(junk, Some(1))]).unwrap();
    s.upsert_message(&acc.id, &mine("3", "eve@evil.test"), &[(inbox, Some(1))]).unwrap();
    let evidence: Vec<String> = s.correspondence_evidence().unwrap().into_iter().map(|e| e.0).collect();
    let known: Vec<String> = s.correspondents().unwrap().into_iter().map(|e| e.0).collect();
    for list in [&evidence, &known] {
        assert!(list.contains(&"bob@x.test".to_string()));
        assert!(!list.iter().any(|e| e.ends_with("@evil.test")), "{list:?}");
    }
    assert_eq!(s.person_summary(&["mallory@evil.test".into()]).unwrap().sent, 0);
}

#[test]
fn deleted_messages_free_their_stored_copies() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let mb = s.upsert_mailbox(&acc.id, "INBOX", "Inbox", "INBOX", MailboxRole::Inbox, false).unwrap();
    let other = s.upsert_mailbox(&acc.id, "Other", "Other", "Other", MailboxRole::Custom, false).unwrap();
    let with_blob = |rid: &str, mid: &str| MessageRecord { blob_hash: Some(format!("h{rid}")), ..rec(rid, mid, None, "x", 0) };
    let (a, _) = s.upsert_message(&acc.id, &with_blob("1", "m1"), &[(mb.clone(), Some(1))]).unwrap();
    let (b, _) = s.upsert_message(&acc.id, &with_blob("2", "m2"), &[(mb.clone(), Some(2)), (other.clone(), Some(1))]).unwrap();
    s.upsert_message(&acc.id, &with_blob("3", "m3"), &[(other.clone(), Some(2))]).unwrap();
    s.upsert_message(&acc.id, &with_blob("4", "m4"), &[(mb.clone(), Some(3))]).unwrap();
    s.delete_message(&a).unwrap();
    assert_eq!(s.take_freed_blobs(), vec!["h1"]);
    // Still in another mailbox: kept.
    s.remove_from_mailbox(&b, &mb).unwrap();
    assert!(s.take_freed_blobs().is_empty());
    s.remove_from_mailbox(&b, &other).unwrap();
    assert_eq!(s.take_freed_blobs(), vec!["h2"]);
    s.remove_mailboxes(&acc.id, std::slice::from_ref(&other)).unwrap();
    assert_eq!(s.take_freed_blobs(), vec!["h3"]);
    s.delete_account(&acc.id).unwrap();
    assert_eq!(s.take_freed_blobs(), vec!["h4"]);
    assert!(!s.blob_in_use("h4").unwrap());
}

#[test]
fn only_failed_or_waiting_ops_can_be_retried() {
    let s = Store::open_in_memory().unwrap();
    let acc = s.insert_account(&AccountConfig::Gmail { email: "a@x.test".into(), display_name: "A".into() }).unwrap();
    let id = s.enqueue_op(&acc.id, "send", &serde_json::json!({}), "user").unwrap();
    s.op_sending(&id).unwrap();
    s.op_reset(&id).unwrap();
    assert!(s.due_ops(&acc.id).unwrap().is_empty(), "a send in flight isn't put back in the queue");
}

#[test]
fn data_errors_are_told_apart_from_database_failures() {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE t (x TEXT UNIQUE)").unwrap();
    c.execute("INSERT INTO t VALUES ('a')", []).unwrap();
    let dup = anyhow::Error::from(c.execute("INSERT INTO t VALUES ('a')", []).unwrap_err());
    assert!(is_data_error(&dup));
    let many = vec!["?"; 40_000].join(",");
    let too_many = anyhow::Error::from(c.prepare(&format!("SELECT 1 WHERE 1 IN ({many})")).unwrap_err());
    assert!(is_data_error(&too_many), "{too_many}");
    let busy = anyhow::Error::from(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None)).context("upsert");
    assert!(!is_data_error(&busy));
    assert!(!is_data_error(&anyhow!("store poisoned")));
}
