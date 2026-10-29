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
