//! Two-way address book sync: linking, pushing edits, conflicts, and merged cards.

mod support;

use support::contacts::*;

#[tokio::test(flavor = "multi_thread")]
async fn address_book_sync_links_pushes_and_merges_conflicts() {
    let (engine, server, account, dir) = setup(true).await;
    engine.contacts_pass().unwrap();
    // The server has Dave, with a phone number: his card absorbs the automatic contact.
    server.put(&format!("{BOOK}dave.vcf"), "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:dave\r\nFN:Dave J.\r\nEMAIL:dave@x.test\r\nTEL;TYPE=cell:+1 555 0100\r\nX-CUSTOM:keep me\r\nEND:VCARD\r\n");
    sync_contacts(&engine, &account).await;
    wait_for(|| engine.store().card_by_remote(&engine.store().list_address_books(None).unwrap().first().map(|b| b.book.id.clone()).unwrap_or_default(), &format!("{BOOK}dave.vcf")).ok().flatten().is_some()).await;
    let list = contacts(&engine, &Caller::User).await;
    assert_eq!(list.len(), 2, "linked, not duplicated: {list:?}");
    let dave = by_email(&list, "dave@x.test").unwrap().clone();
    assert_eq!(dave.fields.name, "Dave J.");
    assert_eq!(dave.origin, ContactOrigin::Remote);
    assert_eq!(dave.sources.len(), 1);
    let book = dave.sources[0].address_book_id.clone();
    engine.mirror_pass(true).unwrap();
    let id = dave.id.clone().unwrap();
    let file = std::fs::read_to_string(dir.path().join(format!("mirror/contacts/people/dave-j-{}.md", &id[id.len() - 8..]))).unwrap();
    assert!(file.contains("accounts: [\"me@x.test\"]\n") && file.contains("- saved in: me@x.test › "), "{file}");

    // A local edit is pushed, keeping properties Satchel doesn't edit.
    let mut e = dave.clone();
    e.fields.title = "Chef".into();
    save(&engine, e, None).await;
    let href = format!("{BOOK}dave.vcf");
    wait_for(|| server.card(&href).is_some_and(|f| f.title == "Chef")).await;
    assert!(server.cards.lock().unwrap()[&href].1.contains("X-CUSTOM:keep me"));

    // Both sides change different fields at once: the push conflicts, merges, and wins where it changed.
    let mut theirs = server.card(&href).unwrap();
    theirs.organization = "Bistro".into();
    let current = server.cards.lock().unwrap()[&href].1.clone();
    server.put(&href, &satchel_ical::vcard::build(Some(&current), "dave", &theirs));
    let mut mine: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::GetContact { contact_id: dave.id.clone().unwrap() }).await.unwrap()).unwrap();
    mine.fields.notes = "Allergic to nuts".into();
    save(&engine, mine, None).await;
    wait_for(|| server.card(&href).is_some_and(|f| f.notes == "Allergic to nuts")).await;
    let f = server.card(&href).unwrap();
    assert_eq!((f.organization.as_str(), f.title.as_str()), ("Bistro", "Chef"));
    let here: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::GetContact { contact_id: dave.id.clone().unwrap() }).await.unwrap()).unwrap();
    assert_eq!(here.fields.organization, "Bistro");

    // A new local contact saved to the address book is created there.
    let mut erin = Contact::default();
    erin.fields.name = "Erin".into();
    erin.fields.emails = vec![ContactEmail { email: "erin@x.test".into(), label: None }];
    let erin = save(&engine, erin, Some(book.clone())).await;
    wait_for(|| server.cards.lock().unwrap().values().any(|(_, v)| v.contains("FN:Erin"))).await;
    sync_contacts(&engine, &account).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(contacts(&engine, &Caller::User).await.len(), 3, "the server copy is recognized, not duplicated");

    // Deleting here deletes there; deleting there hides it here for good.
    engine.handle(&Caller::User, Command::DeleteContact { contact_id: erin.id.unwrap() }).await.unwrap();
    wait_for(|| server.ops.lock().unwrap().iter().any(|(k, _)| k == "delete")).await;
    server.cards.lock().unwrap().remove(&href);
    sync_contacts(&engine, &account).await;
    wait_for(|| engine.store().get_contact(dave.id.as_deref().unwrap()).unwrap().is_some_and(|c| c.deleted)).await;
    engine.contacts_pass().unwrap();
    let left: Vec<String> = contacts(&engine, &Caller::User).await.into_iter().map(|c| c.fields.name).collect();
    assert_eq!(left, vec!["Bob Smith"]);
    let _: Value = engine.handle(&Caller::User, Command::ListAddressBooks).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_without_contacts_access_asks_to_reconnect_instead_of_failing() {
    let (engine, server, account, _dir) = setup(true).await;
    server.scope_missing.store(true, std::sync::atomic::Ordering::SeqCst);
    sync_contacts(&engine, &account).await;
    let get = || engine.store().get_account(&account).unwrap().unwrap();
    wait_for(|| get().contacts_need_reconnect).await;
    wait_for(|| get().sync.state == SyncState::Idle).await;
    assert!(get().sync.error.is_none());
    server.scope_missing.store(false, std::sync::atomic::Ordering::SeqCst);
    sync_contacts(&engine, &account).await;
    wait_for(|| !get().contacts_need_reconnect).await;
}


#[tokio::test(flavor = "multi_thread")]
async fn merging_links_cards_and_edits_change_only_what_changed() {
    let (engine, server, account, _dir) = setup(true).await;
    engine.contacts_pass().unwrap();
    let (a, b) = (format!("{BOOK}work.vcf"), format!("{BOOK}home.vcf"));
    server.put(&a, "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:work\r\nFN:Pat Lee\r\nEMAIL:pat@work.test\r\nTEL:+1 555 0001\r\nPHOTO:data:abc\r\nEND:VCARD\r\n");
    server.put(&b, "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:home\r\nFN:Pat\r\nEMAIL:pat@home.test\r\nTEL:+1 555 0002\r\nEND:VCARD\r\n");
    sync_contacts(&engine, &account).await;
    wait_for(|| engine.store().contacts_by_email("pat@home.test").is_ok_and(|v| !v.is_empty()) && engine.store().contacts_by_email("pat@work.test").is_ok_and(|v| !v.is_empty())).await;
    let list = contacts(&engine, &Caller::User).await;
    let ids = vec![by_email(&list, "pat@work.test").unwrap().id.clone().unwrap(), by_email(&list, "pat@home.test").unwrap().id.clone().unwrap()];
    let merged: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::MergeContacts { contact_ids: ids }).await.unwrap()).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(server.ops.lock().unwrap().is_empty(), "merging changes nothing on the server");
    assert_eq!(server.cards.lock().unwrap().len(), 2);

    let mut e = merged;
    e.fields.title = "Engineer".into();
    save(&engine, e, None).await;
    wait_for(|| server.card(&a).is_some_and(|f| f.title == "Engineer") && server.card(&b).is_some_and(|f| f.title == "Engineer")).await;
    let (work, home) = (server.card(&a).unwrap(), server.card(&b).unwrap());
    let phones = |f: &ContactFields| f.phones.iter().map(|p| p.number.clone()).collect::<Vec<_>>();
    assert_eq!(phones(&work), vec!["+1 555 0001"], "each card keeps its own numbers");
    assert_eq!(phones(&home), vec!["+1 555 0002"]);
    assert_eq!(work.name, "Pat Lee");
    assert!(server.cards.lock().unwrap()[&a].1.contains("PHOTO:data:abc"));
}
