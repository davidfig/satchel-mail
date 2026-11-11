//! Search, new-mail notifications, and folders that keep their mail through renames and moves.

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn full_text_search_combines_with_filters() {
    let (engine, _fake, account, _dir) = setup().await;
    while engine.index_batch(500).unwrap() > 0 {}
    let search = |q: &str| {
        let engine = engine.clone();
        let q = q.to_string();
        async move {
            let v = engine.handle(&Caller::User, Command::ListThreads { query: ThreadQuery { search: Some(q.clone()), ..Default::default() } }).await.unwrap();
            let n = engine.handle(&Caller::User, Command::CountThreads { query: ThreadQuery { search: Some(q), ..Default::default() } }).await.unwrap();
            (inbox_threads(v), n["total"].as_u64().unwrap())
        }
    };
    let (t, n) = search("kickoff").await;
    assert_eq!((t.len(), n), (1, 1), "subject match groups into one thread");
    let (t, _) = search("\"body of invoice\"").await;
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].subject, "Invoice");
    let (_, n) = search("kick").await;
    assert_eq!(n, 1, "prefix match");
    let (_, n) = search("from:carol kickoff").await;
    assert_eq!(n, 1);
    let (_, n) = search("from:bill kickoff").await;
    assert_eq!(n, 0);
    let (_, n) = search("nothing-like-this").await;
    assert_eq!(n, 0);
    let (_, n) = search("evil").await;
    assert_eq!(n, 1, "participants are searchable");

    // Deleting a message removes it from search (via the delete trigger).
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    engine.store().delete_message(&invoice).unwrap();
    let (_, n) = search("invoice").await;
    assert_eq!(n, 0);
}


#[tokio::test(flavor = "multi_thread")]
async fn new_inbox_mail_raises_one_notification() {
    let (engine, fake, account, _dir) = setup().await;
    // The first sync stored mail without announcing it.
    wait_for(|| engine.store().get_account(&account).unwrap().unwrap().sync.last_success.is_some()).await;
    let mut events = engine.subscribe();
    let fresh = |id: &str, from: &str, subject: &str| {
        format!("From: {from}\r\nTo: me@x.test\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.test>\r\nDate: {}\r\n\r\nHi\r\n", chrono::Utc::now().to_rfc2822()).into_bytes()
    };
    deliver(&fake, "n1", fresh("n1", "Dana <dana@x.test>", "Hike photos"), "INBOX");
    deliver(&fake, "n2", fresh("n2", "Ads <ads@x.test>", "Sale"), "Junk");
    // Dated days ago, like a backfill.
    deliver(&fake, "n3", raw("n3", "Old <old@x.test>", "From the archive", None), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    let messages = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::NewMail { messages, .. }) = events.recv().await {
                return messages;
            }
        }
    })
    .await
    .expect("a new_mail event");
    let subjects: Vec<&str> = messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Hike photos"]);
    assert_eq!(messages[0].from.as_ref().unwrap().email, "dana@x.test");
}


#[tokio::test(flavor = "multi_thread")]
async fn folders_are_created_renamed_and_deleted_without_losing_mail() {
    let (engine, fake, account, _dir) = setup().await;
    let user = Caller::User;
    let create = |name: &str, parent: Option<&Mailbox>| Command::CreateMailbox { account_id: account.clone(), name: name.into(), parent_id: parent.map(|p| p.id.clone()) };

    // A new folder is usable at once: a move into it waits for the server to create it.
    let projects: Mailbox = serde_json::from_value(engine.handle(&user, create("Projects", None)).await.unwrap()).unwrap();
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    engine.handle(&user, Command::Move { message_ids: vec![invoice.clone()], mailbox_id: projects.id.clone() }).await.unwrap();
    wait_for(|| fake.moves.lock().unwrap().contains(&"F-Projects".to_string())).await;
    assert_eq!(engine.store().mailbox_remote_id(&projects.id).unwrap().as_deref(), Some("F-Projects"));

    let y2026: Mailbox = serde_json::from_value(engine.handle(&user, create(" 2026 ", Some(&projects))).await.unwrap()).unwrap();
    assert_eq!((y2026.name.as_str(), y2026.path.as_str()), ("2026", "Projects/2026"));

    // Mistakes are refused before anything is queued.
    for bad in [create("projects", None), create("a/b", None), create("  ", None)] {
        assert_eq!(engine.handle(&user, bad).await.unwrap_err().code, ErrorCode::InvalidInput);
    }
    let inbox = folder(&engine, "INBOX").unwrap();
    let into_itself = Command::UpdateMailbox { mailbox_id: projects.id.clone(), name: "Projects".into(), parent_id: Some(y2026.id.clone()) };
    for bad in [into_itself, Command::UpdateMailbox { mailbox_id: inbox.id.clone(), name: "In".into(), parent_id: None }, Command::DeleteMailbox { mailbox_id: projects.id.clone() }] {
        assert_eq!(engine.handle(&user, bad).await.unwrap_err().code, ErrorCode::InvalidInput);
    }
    let a = agent(&engine, AgentPolicy::default()).await;
    assert_eq!(engine.handle(&a, create("Agent", None)).await.unwrap_err().code, ErrorCode::Forbidden);

    // Renaming takes the folders inside along, here and on the server.
    engine.handle(&user, Command::UpdateMailbox { mailbox_id: projects.id.clone(), name: "Work".into(), parent_id: None }).await.unwrap();
    assert_eq!(folder(&engine, "Work/2026").map(|m| m.id), Some(y2026.id.clone()));
    wait_for(|| engine.store().mailbox_remote_id(&y2026.id).unwrap().as_deref() == Some("F-Work/2026")).await;
    assert_eq!(engine.store().mailbox_remote_id(&projects.id).unwrap().as_deref(), Some("F-Work"));

    // Deleting a folder moves its mail to Trash first.
    engine.handle(&user, Command::DeleteMailbox { mailbox_id: y2026.id.clone() }).await.unwrap();
    let r = engine.handle(&user, Command::DeleteMailbox { mailbox_id: projects.id.clone() }).await.unwrap();
    assert_eq!(r["moved_to_trash"], 1);
    let trash = folder(&engine, "Trash").unwrap();
    assert_eq!(engine.store().mailbox_message_ids(&trash.id).unwrap(), vec![invoice]);
    wait_for(|| fake.folders.lock().unwrap().is_empty()).await;
    engine.handle(&user, Command::SyncNow { account_id: None }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(folder(&engine, "Work").is_none() && folder(&engine, "Work/2026").is_none());
    assert_eq!(fake.moves.lock().unwrap().last().map(String::as_str), Some("Trash"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_move_finds_the_message_where_the_first_put_it() {
    let (engine, fake, account, _dir) = setup().await;
    fake.rekey_moves.store(true, std::sync::atomic::Ordering::Relaxed);
    let user = Caller::User;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let archive = folder(&engine, "Archive").unwrap();
    let junk = folder(&engine, "Junk").unwrap();
    // Both queued before either reaches the server.
    engine.handle(&user, Command::Move { message_ids: vec![invoice.clone()], mailbox_id: archive.id.clone() }).await.unwrap();
    engine.handle(&user, Command::Move { message_ids: vec![invoice.clone()], mailbox_id: junk.id.clone() }).await.unwrap();
    wait_for(|| fake.moved_ids.lock().unwrap().len() == 2).await;
    let ids = fake.moved_ids.lock().unwrap().clone();
    assert_eq!(ids[0], vec!["r3".to_string()]);
    assert_eq!(ids[1], vec![format!("Archive#{invoice}")], "the second move uses the id the first one gave it");
}

#[tokio::test(flavor = "multi_thread")]
async fn smart_folders_search_every_folder() {
    let (engine, _fake, account, _dir) = setup().await;
    while engine.index_batch(500).unwrap() > 0 {}
    let user = Caller::User;
    let save = |name: &str, query: &str| Command::SaveSmartFolder { folder: SmartFolder { name: name.into(), query: query.into(), badge: SmartBadge::Total, ..Default::default() } };
    let folders = |caller: Caller| {
        let engine = engine.clone();
        async move { serde_json::from_value::<Vec<SmartFolder>>(engine.handle(&caller, Command::ListSmartFolders).await.unwrap()).unwrap() }
    };
    let threads = |id: &str, search: Option<&str>| {
        let engine = engine.clone();
        let query = ThreadQuery { smart_folder_id: Some(id.into()), search: search.map(String::from), ..Default::default() };
        async move { inbox_threads(engine.handle(&Caller::User, Command::ListThreads { query }).await.unwrap()) }
    };

    let people: SmartFolder = serde_json::from_value(engine.handle(&user, save("People", "-from:bill")).await.unwrap()).unwrap();
    let bills: SmartFolder = serde_json::from_value(engine.handle(&user, save("Bills", "from:bill")).await.unwrap()).unwrap();
    let t = threads(&people.id, None).await;
    assert_eq!(t.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>(), ["Kickoff"]);
    assert!(threads(&people.id, Some("invoice")).await.is_empty(), "a search narrows the folder");
    assert_eq!(folders(user.clone()).await.iter().map(|f| (f.name.as_str(), f.count)).collect::<Vec<_>>(), [("People", 1), ("Bills", 1)]);

    // Trash is left out unless the folder asks for it.
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    engine.handle(&user, Command::Trash { message_ids: vec![invoice] }).await.unwrap();
    assert!(threads(&bills.id, None).await.is_empty());
    let mut anywhere = bills.clone();
    anywhere.query = "from:bill in:anywhere".into();
    engine.handle(&user, Command::SaveSmartFolder { folder: anywhere }).await.unwrap();
    assert_eq!(threads(&bills.id, None).await.len(), 1);

    // Names are unique, and a folder has to search for something.
    assert!(engine.handle(&user, save("people", "is:unread")).await.is_err());
    assert!(engine.handle(&user, save("Empty", "  ... ")).await.is_err());

    // Agents can read smart folders but not change them.
    let bot = agent(&engine, AgentPolicy::default()).await;
    assert_eq!(folders(bot.clone()).await.len(), 2);
    assert!(engine.handle(&bot, save("Mine", "is:unread")).await.is_err());
    assert!(engine.handle(&bot, Command::DeleteSmartFolder { id: people.id.clone() }).await.is_err());

    engine.handle(&user, Command::ReorderSmartFolders { ids: vec![bills.id.clone()] }).await.unwrap();
    assert_eq!(folders(user.clone()).await[0].name, "Bills");
    engine.handle(&user, Command::DeleteSmartFolder { id: people.id }).await.unwrap();
    assert_eq!(folders(user).await.len(), 1);
}

/// A conversation where Zed wrote first, in the Inbox, and I replied in Archive.
async fn zeds_thread(engine: &Arc<Engine>, fake: &Fake, account: &str) -> (String, String) {
    deliver(fake, "p1", raw("p1", "Zed <zed@x.test>", "Plan", None), "INBOX");
    deliver(fake, "p2", raw("p2", "Me <me@x.test>", "Re: Plan", Some("p1")), "Archive");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(account, "p2").unwrap().is_some()).await;
    let id = |rid| engine.store().message_id_by_remote(account, rid).unwrap().unwrap();
    (id("p1"), id("p2"))
}

async fn save_folder(engine: &Arc<Engine>, name: &str, query: &str, badge: SmartBadge) -> SmartFolder {
    let folder = SmartFolder { name: name.into(), query: query.into(), badge, ..Default::default() };
    serde_json::from_value(engine.handle(&Caller::User, Command::SaveSmartFolder { folder }).await.unwrap()).unwrap()
}

async fn subjects(engine: &Arc<Engine>, caller: &Caller, search: &str) -> Vec<String> {
    let query = ThreadQuery { search: Some(search.into()), ..Default::default() };
    inbox_threads(engine.handle(caller, Command::ListThreads { query }).await.unwrap()).into_iter().map(|t| t.subject).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn negated_terms_leave_out_the_whole_thread() {
    let (engine, fake, account, _dir) = setup().await;
    let (zeds, mine) = zeds_thread(&engine, &fake, &account).await;
    let user = Caller::User;
    let has_plan = |v: Vec<String>| v.iter().any(|s| s == "Plan");
    // Each search has a message in the thread that would match on its own.
    assert!(!has_plan(subjects(&engine, &user, "-from:zed").await), "my reply doesn't bring Zed's thread back");
    assert!(!has_plan(subjects(&engine, &user, "-in:inbox").await), "my reply elsewhere doesn't take it out of the Inbox");
    assert!(has_plan(subjects(&engine, &user, "from:zed").await));

    // Read means all of it is read.
    engine.handle(&user, Command::SetFlags { message_ids: vec![zeds], seen: Some(true), flagged: None }).await.unwrap();
    assert!(!has_plan(subjects(&engine, &user, "is:read").await));
    assert!(has_plan(subjects(&engine, &user, "is:unread").await));

    // The badge counts the conversation, though Zed's own message is read.
    save_folder(&engine, "From Zed", "from:zed", SmartBadge::Unread).await;
    let folders = |caller: Caller| {
        let engine = engine.clone();
        async move { serde_json::from_value::<Vec<SmartFolder>>(engine.handle(&caller, Command::ListSmartFolders).await.unwrap()).unwrap() }
    };
    assert_eq!(folders(user.clone()).await[0].count, 1);
    engine.handle(&user, Command::SetFlags { message_ids: vec![mine], seen: Some(true), flagged: None }).await.unwrap();
    assert_eq!(folders(user.clone()).await[0].count, 0);
    assert!(has_plan(subjects(&engine, &user, "is:read").await));
}

#[tokio::test(flavor = "multi_thread")]
async fn negated_terms_cant_reveal_mail_outside_an_agents_date_limit() {
    let (engine, fake, account, _dir) = setup().await;
    // Bob's message is older than the agent's limit; only the reply is inside it.
    let reply = String::from_utf8(raw("q2", "Carol <carol@x.test>", "Re: Old plan", Some("q1"))).unwrap().replace("Tue, 20 Oct 2026 10:00:00 +0000", &chrono::Utc::now().to_rfc2822());
    deliver(&fake, "q1", raw("q1", "Secret <secret@x.test>", "Old plan", None), "INBOX");
    deliver(&fake, "q2", reply.into_bytes(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "q2").unwrap().is_some()).await;
    let bot = agent(&engine, AgentPolicy { since_days: Some(1), ..AgentPolicy::default() }).await;
    let plan = |v: Vec<String>| v.iter().any(|s| s.contains("Old plan"));
    assert!(plan(subjects(&engine, &bot, "from:carol").await));
    // If the old message counted, this would answer "does secret@ write in this thread?".
    assert!(plan(subjects(&engine, &bot, "-from:secret").await));
    assert!(!plan(subjects(&engine, &Caller::User, "-from:secret").await));
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_limited_to_some_accounts_get_no_smart_folders() {
    let (engine, _fake, account, _dir) = setup().await;
    let folder = save_folder(&engine, "Lawyer", "from:lawyer@x.test", SmartBadge::Total).await;
    let everything = agent(&engine, AgentPolicy { account_ids: Some(vec![account.clone()]), ..AgentPolicy::default() }).await;
    let list = |caller: Caller| {
        let engine = engine.clone();
        async move { engine.handle(&caller, Command::ListSmartFolders).await.unwrap().as_array().unwrap().len() }
    };
    assert_eq!(list(everything).await, 1, "listing every account is no limit");

    let limited = agent(&engine, AgentPolicy { account_ids: Some(vec!["another".into()]), ..AgentPolicy::default() }).await;
    assert_eq!(list(limited.clone()).await, 0);
    let query = ThreadQuery { smart_folder_id: Some(folder.id), ..Default::default() };
    assert!(engine.handle(&limited, Command::ListThreads { query }).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn one_bad_smart_folder_doesnt_take_down_the_list() {
    let (engine, _fake, _account, _dir) = setup().await;
    let long = format!("subject:{}", "x".repeat(60_000));
    let refused = SmartFolder { name: "Long".into(), query: long.clone(), ..Default::default() };
    assert!(engine.handle(&Caller::User, Command::SaveSmartFolder { folder: refused.clone() }).await.is_err());
    // One saved before there was a limit: its count fails (too long for LIKE).
    engine.store().save_smart_folder(&refused).unwrap();
    save_folder(&engine, "Everything", "-from:nobody", SmartBadge::Total).await;
    let v = engine.handle(&Caller::User, Command::ListSmartFolders).await.unwrap();
    let folders: Vec<SmartFolder> = serde_json::from_value(v).unwrap();
    assert_eq!(folders.iter().map(|f| f.count > 0).collect::<Vec<_>>(), [false, true]);
}

/// One message stored in two folders (a note to self lands in Sent and the
/// Inbox; a server may keep a copy in Trash). The reader shows one copy, so
/// acting on it must reach the others.
async fn two_copies(fake: &Fake, engine: &Arc<Engine>, account: &str, a: &str, b: &str) -> (String, String) {
    let note = raw("self", "Me <me@x.test>", "Note to self", None);
    deliver(fake, "c1", note.clone(), a);
    deliver(fake, "c2", note, b);
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(account, "c2").unwrap().is_some()).await;
    let id = |rid| engine.store().message_id_by_remote(account, rid).unwrap().unwrap();
    (id("c1"), id("c2"))
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_message_moves_every_copy_and_keeps_the_one_in_trash() {
    let (engine, fake, account, _dir) = setup().await;
    let (in_trash, archived) = two_copies(&fake, &engine, &account, "Trash", "Archive").await;
    // The copy on screen happens to be the one already in Trash.
    engine.handle(&Caller::User, Command::Trash { message_ids: vec![in_trash.clone()] }).await.unwrap();
    assert!(in_role(&engine, &account, "c2", MailboxRole::Trash), "the other copy goes to Trash");
    assert!(!in_role(&engine, &account, "c2", MailboxRole::Archive));
    assert!(engine.store().message_ref(&in_trash).unwrap().is_some(), "nothing is deleted forever while a copy was elsewhere");
    // Deleting again, now that every copy is in Trash, deletes for good.
    engine.handle(&Caller::User, Command::Trash { message_ids: vec![archived.clone()] }).await.unwrap();
    assert!(engine.store().message_ref(&in_trash).unwrap().is_none());
    assert!(engine.store().message_ref(&archived).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_takes_every_copy_out_of_the_inbox_and_flags_reach_them_all() {
    let (engine, fake, account, _dir) = setup().await;
    let (in_inbox, in_junk) = two_copies(&fake, &engine, &account, "INBOX", "Junk").await;
    engine.handle(&Caller::User, Command::SetFlags { message_ids: vec![in_junk.clone()], seen: Some(true), flagged: None }).await.unwrap();
    assert!(engine.store().message_ref(&in_inbox).unwrap().unwrap().flags.seen, "reading one copy reads them all");
    engine.handle(&Caller::User, Command::Archive { message_ids: vec![in_junk] }).await.unwrap();
    assert!(!in_role(&engine, &account, "c1", MailboxRole::Inbox), "the Inbox copy is archived");
    assert!(in_role(&engine, &account, "c1", MailboxRole::Archive));
}

/// A message with one part: `disposition` (with its filename) and `cid`.
fn with_part(id: &str, subject: &str, mime: &str, disposition: &str, cid: Option<&str>) -> Vec<u8> {
    let cid = cid.map(|c| format!("Content-ID: <{c}>\r\n")).unwrap_or_default();
    format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.test>\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
         --b\r\nContent-Type: {mime}\r\nContent-Disposition: {disposition}\r\n{cid}Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n",
        chrono::Utc::now().to_rfc2822()
    )
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_found_by_kind_and_name() {
    let (engine, fake, account, _dir) = setup().await;
    let pptx = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
    let mail = [
        ("a1", with_part("a1", "Deck", pptx, "attachment; filename=\"Q3 Plan_final.PPTX\"", None)),
        // Apple Mail sends documents inline; they're still attachments.
        ("a2", with_part("a2", "Bill 42", "application/pdf", "inline; filename=\"invoice-42.pdf\"", Some("pdf1"))),
        // A logo the signature shows in place isn't.
        ("a3", with_part("a3", "Logo", "image/png", "inline; filename=\"logo.png\"", Some("logo1"))),
    ];
    for (id, raw) in &mail {
        deliver(&fake, id, raw.clone(), "INBOX");
    }
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| ["a1", "a2", "a3"].iter().all(|id| engine.store().message_id_by_remote(&account, id).ok().flatten().is_some())).await;
    let user = Caller::User;
    let found = |q: &'static str| {
        let engine = engine.clone();
        let user = user.clone();
        async move {
            let mut s = subjects(&engine, &user, q).await;
            s.sort();
            s
        }
    };
    assert_eq!(found("has:presentation").await, ["Deck"]);
    assert_eq!(found("has:pptx").await, ["Deck"]);
    assert_eq!(found("has:pdf").await, ["Bill 42"]);
    assert_eq!(found("has:pdf,pptx").await, ["Bill 42", "Deck"]);
    assert!(found("has:image").await.is_empty(), "signature logos don't count");
    assert_eq!(found("filename:plan_final").await, ["Deck"], "`_` is a character, not a wildcard");
    assert!(found("filename:plan%final").await.is_empty());
    assert_eq!(found("filename:invoice,plan").await, ["Bill 42", "Deck"]);
    assert!(found("-has:pdf").await.contains(&"Deck".to_string()));
    assert!(!found("-has:pdf").await.contains(&"Bill 42".to_string()));
}
