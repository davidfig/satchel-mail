//! Contacts from the mail you exchange: automatic contacts, user edits, merging, the person view, and agent access.

mod support;

use support::contacts::*;

#[tokio::test(flavor = "multi_thread")]
async fn people_you_write_to_become_contacts_and_user_edits_win() {
    let (engine, _, _, _dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    let list = contacts(&engine, &Caller::User).await;
    let names: Vec<&str> = list.iter().map(|c| c.fields.name.as_str()).collect();
    assert_eq!(names, vec!["Bob Smith", "Dave Jones"], "only people I wrote to or answered");
    let dave = by_email(&list, "dave@x.test").unwrap().clone();
    assert_eq!(dave.origin, ContactOrigin::Auto);
    assert_eq!(dave.message_count, 2, "his message and my reply");

    // Rename Dave and delete Bob; a later pass must change neither.
    let mut edited = dave.clone();
    edited.fields.name = "David".into();
    let edited = save(&engine, edited, None).await;
    assert_eq!(edited.origin, ContactOrigin::Local);
    let bob = by_email(&list, "bob@x.test").unwrap().id.clone().unwrap();
    engine.handle(&Caller::User, Command::DeleteContact { contact_id: bob }).await.unwrap();
    assert_eq!(engine.contacts_pass().unwrap(), 0);
    let list = contacts(&engine, &Caller::User).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].fields.name, "David");

    // A contact needs only an address.
    let mut fresh = Contact::default();
    fresh.fields.emails = vec![ContactEmail { email: "x@y.test".into(), label: None }];
    save(&engine, fresh, None).await;
    assert_eq!(contacts(&engine, &Caller::User).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_automatic_contact_gives_way_to_another_with_its_address() {
    let (engine, _, _, _dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    // A server card for Dave stored while the pass was running, so it
    // didn't take over his automatic contact.
    let fields = ContactFields { name: "Dave J.".into(), emails: vec![ContactEmail { email: "dave@x.test".into(), label: None }], ..Default::default() };
    engine.store().insert_contact(&fields, ContactOrigin::Remote).unwrap();
    let daves = |list: Vec<Contact>| list.into_iter().filter(|c| c.fields.emails.iter().any(|e| e.email == "dave@x.test")).collect::<Vec<_>>();
    assert_eq!(daves(contacts(&engine, &Caller::User).await).len(), 2);
    engine.contacts_pass().unwrap();
    let left = daves(contacts(&engine, &Caller::User).await);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].origin, ContactOrigin::Remote);
    assert_eq!(engine.contacts_pass().unwrap(), 0, "and nothing comes back");
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_pools_addresses_and_composer_suggests_contacts() {
    let (engine, _, _, _dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    let mut other = Contact::default();
    other.fields.name = "Robert Smith".into();
    other.fields.emails = vec![ContactEmail { email: "rob@work.test".into(), label: Some("work".into()) }];
    other.fields.organization = "Acme".into();
    let other = save(&engine, other, None).await;
    let bob = by_email(&contacts(&engine, &Caller::User).await, "bob@x.test").unwrap().id.clone().unwrap();
    let merged: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::MergeContacts { contact_ids: vec![bob.clone(), other.id.clone().unwrap()] }).await.unwrap()).unwrap();
    assert_eq!(merged.id.as_deref(), Some(bob.as_str()));
    assert_eq!(merged.fields.name, "Bob Smith");
    assert_eq!(merged.fields.organization, "Acme");
    assert_eq!(merged.fields.emails.len(), 2);
    assert_eq!(merged.origin, ContactOrigin::Local);
    assert!(engine.handle(&Caller::User, Command::GetContact { contact_id: other.id.unwrap() }).await.is_err());
    // Merged-away addresses are still claimed: no new automatic contact.
    engine.contacts_pass().unwrap();
    assert_eq!(contacts(&engine, &Caller::User).await.len(), 2);

    let s: Vec<Address> = serde_json::from_value(engine.handle(&Caller::User, Command::SuggestAddresses { prefix: "rob".into() }).await.unwrap()).unwrap();
    assert_eq!(s[0].email, "rob@work.test");
    assert_eq!(s[0].name.as_deref(), Some("Bob Smith"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_person_view_covers_all_of_a_contacts_addresses_and_counts_agree() {
    let (engine, _, _, _dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    let mut dave = by_email(&contacts(&engine, &Caller::User).await, "dave@x.test").unwrap().clone();
    dave.fields.emails.push(ContactEmail { email: "Dave@Home.test".into(), label: Some("home".into()) });
    let dave = save(&engine, dave, None).await;

    // Asking about his other address finds the contact and all its addresses.
    let p: PersonSummary = serde_json::from_value(engine.handle(&Caller::User, Command::GetPerson { emails: vec!["dave@home.test".into()] }).await.unwrap()).unwrap();
    assert_eq!(p.contact_id, dave.id);
    assert_eq!(p.name.as_deref(), Some("Dave Jones"));
    assert_eq!(p.addresses.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), vec!["dave@x.test", "dave@home.test"]);
    assert_eq!((p.received, p.sent, p.threads), (1, 1, 1));
    assert_eq!(dave.message_count, p.received + p.sent, "the contact and the person view count the same mail");

    // Someone who isn't a contact is just their address.
    let p: PersonSummary = serde_json::from_value(engine.handle(&Caller::User, Command::GetPerson { emails: vec!["carol@x.test".into()] }).await.unwrap()).unwrap();
    assert_eq!((p.contact_id, p.received, p.sent), (None, 1, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_read_contacts_and_can_only_propose_changes() {
    let (engine, _, _, dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "A".into(), policy: AgentPolicy::default() }).await.unwrap();
    let agent = Caller::Agent(engine.authenticate_agent(v["token"].as_str().unwrap()).unwrap());
    let list = contacts(&engine, &agent).await;
    assert_eq!(list.len(), 2);
    let dave = by_email(&list, "dave@x.test").unwrap().clone();
    let mut changed = dave.clone();
    changed.fields.emails.push(ContactEmail { email: "dave@evil.test".into(), label: None });
    let err = engine.handle(&agent, Command::SaveContact { contact: changed.clone(), address_book_id: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let p: Proposal = serde_json::from_value(
        engine.handle(&agent, Command::CreateProposal { action: ProposalAction::SaveContact { contact: changed }, rationale: "new address".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("dave@evil.test")), "{:?}", p.warnings);
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id, edited: None }).await.unwrap();
    let after: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::GetContact { contact_id: dave.id.clone().unwrap() }).await.unwrap()).unwrap();
    assert_eq!(after.fields.emails.len(), 2);
    assert_eq!(after.origin, ContactOrigin::Local);

    // Contacts follow the calendar switch for old policies, and can be turned off.
    let old: AgentPolicy = serde_json::from_value(serde_json::json!({ "account_ids": null, "since_days": 30, "allow_attachments": false, "allow_calendar": false, "max_proposals_per_hour": 5 })).unwrap();
    assert!(!old.allow_contacts);
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "B".into(), policy: AgentPolicy { allow_contacts: false, ..Default::default() } }).await.unwrap();
    let blocked = Caller::Agent(engine.authenticate_agent(v["token"].as_str().unwrap()).unwrap());
    assert_eq!(engine.handle(&blocked, Command::ListContacts { search: None, limit: None }).await.unwrap_err().code, ErrorCode::Forbidden);

    // The AI folder lists them, and takes a contact proposal file.
    engine.mirror_pass(true).unwrap();
    let md = std::fs::read_to_string(dir.path().join("mirror/contacts/CONTACTS.md")).unwrap();
    assert!(md.contains("Dave Jones") && md.contains("dave@evil.test"));
    let people = dir.path().join("mirror/contacts/people");
    let files = || -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&people).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        v.sort();
        v
    };
    let id = dave.id.clone().unwrap();
    let dave_file = format!("dave-jones-{}.md", &id[id.len() - 8..]);
    assert!(files().contains(&dave_file), "{:?}", files());
    let one = std::fs::read_to_string(people.join(&dave_file)).unwrap();
    assert!(one.contains(&format!("contact_id: {id}\n")) && one.contains("dave@evil.test") && one.contains("only in Satchel"), "{one}");
    assert!(md.contains(&format!("(people/{dave_file})")));
    // A rename moves the file, and a deleted contact's file goes.
    let mut renamed: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::GetContact { contact_id: id.clone() }).await.unwrap()).unwrap();
    renamed.fields.name = "David Jones".into();
    save(&engine, renamed, None).await;
    let bob = by_email(&list, "bob@x.test").unwrap().id.clone().unwrap();
    engine.handle(&Caller::User, Command::DeleteContact { contact_id: bob }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    assert_eq!(files(), vec![format!("david-jones-{}.md", &id[id.len() - 8..])]);
    std::fs::write(dir.path().join("mirror/proposals/inbox/c.md"), "---\naction: save_contact\nname: Erin\nemails: [erin@x.test]\nwhy: met at lunch\n---\nLikes tea\n").unwrap();
    engine.ingest_dropped_proposals().await;
    let pending: Vec<Proposal> = serde_json::from_value(engine.handle(&Caller::User, Command::ListProposals { status: Some(ProposalStatus::Pending) }).await.unwrap()).unwrap();
    let ProposalAction::SaveContact { contact } = &pending[0].action else { panic!("{:?}", pending[0].action) };
    assert_eq!(contact.fields.notes, "Likes tea");
    assert_eq!(pending[0].summary, "New contact: Erin");
}


#[tokio::test(flavor = "multi_thread")]
async fn agents_edit_and_add_contact_files_as_proposals() {
    let (engine, _, _, dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    engine.mirror_pass(true).unwrap();
    let people = dir.path().join("mirror/contacts/people");
    let processed = dir.path().join("mirror/proposals/processed");
    let all = || async { serde_json::from_value::<Vec<Proposal>>(engine.handle(&Caller::User, Command::ListProposals { status: None }).await.unwrap()).unwrap() };
    let dave = by_email(&contacts(&engine, &Caller::User).await, "dave@x.test").unwrap().clone();
    let id = dave.id.clone().unwrap();
    let file = people.join(format!("dave-jones-{}.md", &id[id.len() - 8..]));

    // Untouched files propose nothing.
    engine.ingest_dropped_proposals().await;
    assert!(all().await.is_empty());

    // The agent edits the header; the user changes another field before it's read,
    // and a pass in between leaves the edit alone.
    let text = std::fs::read_to_string(&file).unwrap();
    let edited = text.replace("name: \"Dave Jones\"\n", "name: \"Dave Jones\"\ntitle: \"Chef\"\nwhy: \"his signature says so\"\n");
    std::fs::write(&file, &edited).unwrap();
    let mut mine = dave.clone();
    mine.fields.organization = "Bistro".into();
    save(&engine, mine, None).await;
    engine.mirror_pass(true).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited, "a pending edit isn't overwritten");
    engine.ingest_dropped_proposals().await;
    let ps = all().await;
    assert_eq!(ps.len(), 1, "{ps:?}");
    let ProposalAction::SaveContact { contact } = &ps[0].action else { panic!("{:?}", ps[0].action) };
    assert_eq!(contact.id.as_deref(), Some(id.as_str()));
    assert_eq!((contact.fields.title.as_str(), contact.fields.organization.as_str()), ("Chef", "Bistro"), "only the agent's change is proposed");
    assert_eq!(ps[0].rationale, "his signature says so");
    let record = std::fs::read_to_string(processed.join(file.file_name().unwrap())).unwrap();
    assert!(record.contains(&format!("proposal: {}", ps[0].id)), "{record}");

    // The person's file comes back from the store.
    engine.mirror_pass(false).unwrap();
    let back = std::fs::read_to_string(&file).unwrap();
    assert!(back.contains("organization: \"Bistro\"") && !back.contains("why:"), "{back}");

    // A new file is a new contact; touching only the reading list is reported back.
    std::fs::write(people.join("frank.md"), "---\nname: Frank Ocean\nemails: [frank@x.test]\nwhy: met at lunch\n---\n\nLikes tea\n").unwrap();
    let bob = by_email(&contacts(&engine, &Caller::User).await, "bob@x.test").unwrap().id.clone().unwrap();
    let bob_file = people.join(format!("bob-smith-{}.md", &bob[bob.len() - 8..]));
    let t = std::fs::read_to_string(&bob_file).unwrap();
    std::fs::write(&bob_file, t.replace("- email: bob@x.test", "- email: bob@other.test")).unwrap();
    engine.ingest_dropped_proposals().await;
    let ps = all().await;
    assert_eq!(ps.len(), 2, "{ps:?}");
    let frank = ps.iter().find(|p| p.summary == "New contact: Frank Ocean").expect("new contact proposal");
    let ProposalAction::SaveContact { contact } = &frank.action else { unreachable!() };
    assert_eq!(contact.fields.notes, "Likes tea");
    assert!(!people.join("frank.md").exists());
    let bob_record = std::fs::read_to_string(processed.join(bob_file.file_name().unwrap())).unwrap();
    assert!(bob_record.contains("status: error") && bob_record.contains("nothing to change"), "{bob_record}");
    engine.mirror_pass(false).unwrap();
    assert!(std::fs::read_to_string(&bob_file).unwrap().contains("- email: bob@x.test"));
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_propose_merging_duplicate_contacts_and_the_user_decides() {
    let (engine, _, _, dir) = setup(false).await;
    engine.contacts_pass().unwrap();
    let dave = by_email(&contacts(&engine, &Caller::User).await, "dave@x.test").unwrap().id.clone().unwrap();
    let mut other = Contact::default();
    other.fields.name = "David Jones".into();
    other.fields.emails = vec![ContactEmail { email: "dave@home.test".into(), label: None }];
    let other = save(&engine, other, None).await.id.unwrap();
    let pending = || async { serde_json::from_value::<Vec<Proposal>>(engine.handle(&Caller::User, Command::ListProposals { status: Some(ProposalStatus::Pending) }).await.unwrap()).unwrap() };

    // Agents can't merge directly, and need contacts shared to propose it.
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "A".into(), policy: AgentPolicy::default() }).await.unwrap();
    let agent = Caller::Agent(engine.authenticate_agent(v["token"].as_str().unwrap()).unwrap());
    let err = engine.handle(&agent, Command::MergeContacts { contact_ids: vec![dave.clone(), other.clone()] }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "B".into(), policy: AgentPolicy { allow_contacts: false, ..Default::default() } }).await.unwrap();
    let blocked = Caller::Agent(engine.authenticate_agent(v["token"].as_str().unwrap()).unwrap());
    let merge = ProposalAction::MergeContacts { contact_ids: vec![dave.clone(), other.clone()] };
    let err = engine.handle(&blocked, Command::CreateProposal { action: merge.clone(), rationale: String::new(), source_thread_ids: vec![] }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let one = ProposalAction::MergeContacts { contact_ids: vec![dave.clone(), dave.clone()] };
    assert!(engine.handle(&agent, Command::CreateProposal { action: one, rationale: String::new(), source_thread_ids: vec![] }).await.is_err());
    assert!(pending().await.is_empty());

    // A proposal file in the AI folder; it waits for the user with warnings.
    engine.mirror_pass(true).unwrap();
    std::fs::write(dir.path().join("mirror/proposals/inbox/m.md"), format!("---\naction: merge_contacts\ncontacts: [{dave}, {other}]\nwhy: same person\n---\n")).unwrap();
    engine.ingest_dropped_proposals().await;
    let ps = pending().await;
    assert_eq!(ps.len(), 1, "{ps:?}");
    let ProposalAction::MergeContacts { contact_ids } = &ps[0].action else { panic!("{:?}", ps[0].action) };
    assert_eq!(contact_ids, &vec![dave.clone(), other.clone()]);
    assert_eq!(ps[0].summary, "Merge 2 contacts into Dave Jones");
    assert_eq!(ps[0].rationale, "same person");
    assert!(ps[0].warnings.iter().any(|w| w.contains("dave@home.test")), "{:?}", ps[0].warnings);
    assert!(ps[0].warnings.iter().any(|w| w.contains("different names")), "{:?}", ps[0].warnings);
    let bad = format!("---\naction: merge_contacts\ncontacts: [{dave}, nobody]\n---\n");
    std::fs::write(dir.path().join("mirror/proposals/inbox/bad.md"), bad).unwrap();
    engine.ingest_dropped_proposals().await;
    let record = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/bad.md")).unwrap();
    assert!(record.contains("status: error") && record.contains("unknown contact"), "{record}");

    // Approving merges them into the first.
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: ps[0].id.clone(), edited: None }).await.unwrap();
    let merged: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::GetContact { contact_id: dave.clone() }).await.unwrap()).unwrap();
    assert_eq!(merged.fields.name, "Dave Jones");
    assert_eq!(merged.fields.emails.iter().map(|e| e.email.as_str()).collect::<Vec<_>>(), vec!["dave@x.test", "dave@home.test"]);
    assert!(engine.handle(&Caller::User, Command::GetContact { contact_id: other.clone() }).await.is_err());

    // A contact changed after the proposal makes it stale.
    let mut third = Contact::default();
    third.fields.emails = vec![ContactEmail { email: "dj@x.test".into(), label: None }];
    let third = save(&engine, third, None).await;
    let p: Proposal = serde_json::from_value(
        engine
            .handle(&agent, Command::CreateProposal { action: ProposalAction::MergeContacts { contact_ids: vec![dave.clone(), third.id.clone().unwrap()] }, rationale: String::new(), source_thread_ids: vec![] })
            .await
            .unwrap(),
    )
    .unwrap();
    let mut t = third.clone();
    t.fields.name = "DJ".into();
    save(&engine, t, None).await;
    let err = engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id, edited: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}
