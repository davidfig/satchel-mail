//! Settings sync between computers through a shared folder (0080).

mod support;

use support::mail::*;

async fn user(e: &Arc<Engine>, cmd: Command) -> Value {
    e.handle(&Caller::User, cmd).await.unwrap()
}

async fn set_theme(e: &Arc<Engine>, theme: &str) {
    let mut s: Settings = serde_json::from_value(user(e, Command::GetSettings).await).unwrap();
    s.theme = theme.into();
    user(e, Command::UpdateSettings { settings: s }).await;
}

fn theme(e: &Arc<Engine>) -> String {
    e.settings().theme
}

async fn pass(e: &Arc<Engine>) -> FolderSync {
    serde_json::from_value(user(e, Command::SyncSettingsNow).await).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_follow_the_person_between_computers() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_string_lossy().to_string();
    let (a, _fa, acc_a, _da) = setup().await;
    let (b, _fb, acc_b, _db) = setup().await;

    // Computer A has settings; B has its own before it joins.
    set_theme(&a, "dark").await;
    let sf = user(&a, Command::SaveSmartFolder { folder: SmartFolder { name: "Bills".into(), query: "invoice".into(), ..Default::default() } }).await;
    let sf_id = sf["id"].as_str().map(String::from).unwrap_or_else(|| sf.as_str().unwrap().to_string());
    user(&a, Command::SetSenderRule { email: "Spammer@Example.com".into(), verdict: Some(SenderVerdict::Spam) }).await;
    user(&a, Command::SetFavoriteSenders { emails: vec!["boss@example.com".into()], favorite: true }).await;
    let made = user(&a, Command::CreateAgent {
        name: "Claude".into(),
        policy: AgentPolicy { account_ids: Some(vec![acc_a.clone()]), allow_calendar: false, ..Default::default() },
    })
    .await;
    let agent_id = made["agent"]["id"].as_str().unwrap().to_string();
    set_theme(&b, "light").await;

    let code = user(&a, Command::EnableSync { folder: path.clone() }).await["code"].as_str().unwrap().to_string();

    // A wrong code is refused before anything changes.
    let wrong = b.handle(&Caller::User, Command::JoinSync { folder: path.clone(), code: "A".repeat(52) }).await;
    assert!(wrong.is_err());
    assert_eq!(theme(&b), "light");

    let st: FolderSync = serde_json::from_value(user(&b, Command::JoinSync { folder: path.clone(), code: code.to_lowercase() }).await).unwrap();
    assert!(st.enabled && st.error.is_none(), "{st:?}");
    assert_eq!(st.devices.len(), 1);

    // What was already there wins over the joining computer's settings.
    assert_eq!(theme(&b), "dark");
    let folders = b.store().list_smart_folders().unwrap();
    assert_eq!(folders.iter().map(|f| (f.id.as_str(), f.name.as_str())).collect::<Vec<_>>(), vec![(sf_id.as_str(), "Bills")]);
    assert_eq!(b.store().sender_rule("spammer@example.com").unwrap(), Some(SenderVerdict::Spam));
    assert_eq!(b.store().list_favorite_senders().unwrap(), vec!["boss@example.com".to_string()]);

    // The agent arrives with its policy, mapped to B's copy of the account,
    // and with no token until one is made here.
    let ag = b.store().get_agent(&agent_id).unwrap().expect("agent synced");
    assert_eq!(ag.name, "Claude");
    assert_eq!(ag.policy.account_ids, Some(vec![acc_b.clone()]));
    assert!(!ag.policy.allow_calendar);
    let t = user(&b, Command::NewAgentToken { agent_id: agent_id.clone() }).await;
    assert!(b.authenticate_agent(t["token"].as_str().unwrap()).is_some());

    // Same account on both, so nothing is offered.
    assert!(st.offered_accounts.is_empty());

    // A's own settings weren't overwritten by B's join.
    pass(&a).await;
    assert_eq!(theme(&a), "dark");

    // Changes on B reach A; revoking on B revokes on A.
    tokio::time::sleep(Duration::from_millis(5)).await;
    set_theme(&b, "system").await;
    user(&b, Command::SaveSmartFolder { folder: SmartFolder { id: sf_id.clone(), name: "Invoices".into(), query: "invoice".into(), ..Default::default() } }).await;
    user(&b, Command::SetSenderRule { email: "spammer@example.com".into(), verdict: None }).await;
    user(&b, Command::RevokeAgent { agent_id: agent_id.clone() }).await;
    pass(&b).await;
    let st = pass(&a).await;
    assert!(st.error.is_none(), "{st:?}");
    assert_eq!(theme(&a), "system");
    assert_eq!(a.store().get_smart_folder(&sf_id).unwrap().unwrap().name, "Invoices");
    assert_eq!(a.store().sender_rule("spammer@example.com").unwrap(), None);
    assert!(a.store().get_agent(&agent_id).unwrap().unwrap().revoked);

    // Settings about this computer stay on it.
    let mut s = a.settings();
    s.initial_sync_days = 7;
    user(&a, Command::UpdateSettings { settings: s }).await;
    pass(&a).await;
    pass(&b).await;
    assert_ne!(b.settings().initial_sync_days, 7);

    // Nothing readable is in the folder.
    for ent in std::fs::read_dir(folder.path()).unwrap() {
        let text = std::fs::read_to_string(ent.unwrap().path()).unwrap();
        for secret in ["Invoices", "boss@example.com", "Claude", "system"] {
            assert!(!text.contains(secret), "{secret} readable in the sync folder");
        }
    }

    // Turning sync off takes B's file out and leaves its settings.
    user(&b, Command::DisableSync).await;
    let st = pass(&a).await;
    assert!(st.devices.is_empty());
    assert_eq!(theme(&b), "system");
    let off: FolderSync = serde_json::from_value(user(&b, Command::GetSync).await).unwrap();
    assert!(!off.enabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_with_a_sync_is_joined_not_restarted() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_string_lossy().to_string();
    let (a, _fa, _, _da) = setup().await;
    let (b, _fb, _, _db) = setup().await;
    user(&a, Command::EnableSync { folder: path.clone() }).await;
    let err = b.handle(&Caller::User, Command::EnableSync { folder: path.clone() }).await.unwrap_err();
    assert!(err.message.contains("join"), "{}", err.message);
    let err = b.handle(&Caller::User, Command::JoinSync { folder: "relative/path".into(), code: "x".into() }).await.unwrap_err();
    assert!(err.message.contains("full path"), "{}", err.message);
}

async fn paired() -> (Arc<Engine>, Arc<Fake>, String, Arc<Engine>, Arc<Fake>, String, Vec<tempfile::TempDir>) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_string_lossy().to_string();
    let (a, fa, acc_a, da) = setup().await;
    let (b, fb, acc_b, db) = setup().await;
    let code = user(&a, Command::EnableSync { folder: path.clone() }).await["code"].as_str().unwrap().to_string();
    user(&b, Command::JoinSync { folder: path, code }).await;
    (a, fa, acc_a, b, fb, acc_b, vec![folder, da, db])
}

fn proposal(e: &Arc<Engine>, id: &str) -> Option<Proposal> {
    e.store().get_proposal(id).unwrap().map(|(p, _)| p)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_is_decided_once_on_any_computer() {
    let (a, fa, acc_a, b, fb, acc_b, _dirs) = paired().await;
    // The agent is made on A; its proposal points at A's ids.
    let ag = agent(&a, AgentPolicy::default()).await;
    let threads = inbox_threads(a.handle(&ag, Command::ListThreads { query: ThreadQuery { role: Some(MailboxRole::Inbox), ..Default::default() } }).await.unwrap());
    let ids = a.store().thread_message_ids(&threads[0].thread_id).unwrap();
    let p: Proposal = serde_json::from_value(
        a.handle(&ag, Command::CreateProposal { action: ProposalAction::Archive { message_ids: ids.clone() }, rationale: "done with it".into(), source_thread_ids: vec![threads[0].thread_id.clone()] })
            .await
            .unwrap(),
    )
    .unwrap();
    pass(&a).await;
    let st = pass(&b).await;
    assert!(st.error.is_none(), "{st:?}");

    // On B it waits for review, pointing at B's copies of the messages.
    let there = proposal(&b, &p.id).expect("proposal synced");
    assert_eq!(there.status, ProposalStatus::Pending);
    assert_eq!(there.rationale, "done with it");
    let ProposalAction::Archive { message_ids } = &there.action else { panic!("{:?}", there.action) };
    let rids = |e: &Arc<Engine>, ids: &[String]| ids.iter().map(|i| e.store().message_sync_ref(i).unwrap().unwrap().1).collect::<Vec<_>>();
    assert_eq!(rids(&b, message_ids), rids(&a, &ids));
    assert!(message_ids.iter().all(|m| b.store().message_sync_ref(m).unwrap().unwrap().0 == acc_b));
    assert_eq!(there.source_thread_ids.len(), 1);
    assert_eq!(there.agent_id, p.agent_id);

    // Approved on B: B archives; A takes the outcome and doesn't archive again.
    user(&b, Command::ApproveProposal { proposal_id: p.id.clone(), edited: None }).await;
    wait_for(|| fb.applied.lock().unwrap().iter().any(|x| x == "move")).await;
    pass(&b).await;
    pass(&a).await;
    assert_eq!(proposal(&a, &p.id).unwrap().status, ProposalStatus::Applied);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!fa.applied.lock().unwrap().iter().any(|x| x == "move"), "A carried it out too");
    assert!(!in_role(&a, &acc_a, &a.store().message_sync_ref(&ids[0]).unwrap().unwrap().1, MailboxRole::Archive));

    // A rejection on A reaches B.
    let p2: Proposal = serde_json::from_value(
        a.handle(&ag, Command::CreateProposal { action: ProposalAction::Flag { message_ids: ids.clone(), flagged: true }, rationale: "".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    pass(&a).await;
    pass(&b).await;
    assert_eq!(proposal(&b, &p2.id).unwrap().status, ProposalStatus::Pending);
    user(&a, Command::RejectProposal { proposal_id: p2.id.clone(), note: Some("no".into()) }).await;
    pass(&a).await;
    pass(&b).await;
    let r = proposal(&b, &p2.id).unwrap();
    assert_eq!((r.status, r.review_note.as_deref()), (ProposalStatus::Rejected, Some("no")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_waits_for_its_message_to_arrive() {
    let (a, fa, acc_a, b, fb, acc_b, _dirs) = paired().await;
    let mail = raw("<late@example.com>", "bob@example.com", "Late one", None);
    deliver(&fa, "late", mail.clone(), "INBOX");
    user(&a, Command::SyncNow { account_id: None }).await;
    wait_for(|| a.store().message_id_by_remote(&acc_a, "late").ok().flatten().is_some()).await;
    let id = a.store().message_id_by_remote(&acc_a, "late").unwrap().unwrap();
    let ag = agent(&a, AgentPolicy::default()).await;
    let p: Proposal = serde_json::from_value(
        a.handle(&ag, Command::CreateProposal { action: ProposalAction::MarkRead { message_ids: vec![id], seen: true }, rationale: "".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    pass(&a).await;
    pass(&b).await;
    assert!(proposal(&b, &p.id).is_none(), "B doesn't have the message yet");

    deliver(&fb, "late", mail, "INBOX");
    user(&b, Command::SyncNow { account_id: None }).await;
    wait_for(|| b.store().message_id_by_remote(&acc_b, "late").ok().flatten().is_some()).await;
    pass(&b).await;
    let there = proposal(&b, &p.id).expect("arrives once the message has");
    let ProposalAction::MarkRead { message_ids, .. } = there.action else { panic!() };
    assert_eq!(message_ids, vec![b.store().message_id_by_remote(&acc_b, "late").unwrap().unwrap()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn contact_proposals_stay_on_their_computer() {
    let (a, _fa, _, b, _fb, _, _dirs) = paired().await;
    let ag = agent(&a, AgentPolicy::default()).await;
    let mut contact = Contact::default();
    contact.fields.name = "Ada".into();
    contact.fields.emails = vec![ContactEmail { email: "ada@example.com".into(), ..Default::default() }];
    let p: Proposal = serde_json::from_value(
        a.handle(&ag, Command::CreateProposal { action: ProposalAction::SaveContact { contact }, rationale: "".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    pass(&a).await;
    pass(&b).await;
    assert!(proposal(&a, &p.id).is_some());
    assert!(proposal(&b, &p.id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decision_beats_a_later_pending_from_elsewhere() {
    let (a, _fa, _, b, fb, _, _dirs) = paired().await;
    let ag = agent(&a, AgentPolicy::default()).await;
    let threads = inbox_threads(a.handle(&ag, Command::ListThreads { query: ThreadQuery { role: Some(MailboxRole::Inbox), ..Default::default() } }).await.unwrap());
    let ids = a.store().thread_message_ids(&threads[0].thread_id).unwrap();
    let p: Proposal = serde_json::from_value(
        a.handle(&ag, Command::CreateProposal { action: ProposalAction::Flag { message_ids: ids.clone(), flagged: true }, rationale: "".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    pass(&a).await;
    pass(&b).await;

    // B approves; before A hears of it, the message changes on A and an
    // approval there finds the proposal stale: a newer, undecided status.
    user(&b, Command::ApproveProposal { proposal_id: p.id.clone(), edited: None }).await;
    wait_for(|| fb.applied.lock().unwrap().iter().any(|x| x == "flags")).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    user(&a, Command::Archive { message_ids: vec![ids[0].clone()] }).await;
    assert!(a.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id.clone(), edited: None }).await.is_err());
    assert_eq!(proposal(&a, &p.id).unwrap().status, ProposalStatus::Stale);
    pass(&a).await;

    // B keeps its decision and sends it out again; A ends up with it.
    pass(&b).await;
    assert_eq!(proposal(&b, &p.id).unwrap().status, ProposalStatus::Applied);
    pass(&a).await;
    assert_eq!(proposal(&a, &p.id).unwrap().status, ProposalStatus::Applied);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_access_to_another_computers_account_is_kept() {
    let (a, _fa, acc_a, b, _fb, acc_b, _dirs) = paired().await;
    let work: Account = serde_json::from_value(
        user(&a, Command::AddAccount { config: AccountConfig::Gmail { email: "work@x.test".into(), display_name: "Work".into() }, credential: Credential::Password { password: "x".into() } }).await,
    )
    .unwrap();
    let made = user(&a, Command::CreateAgent { name: "Claude".into(), policy: AgentPolicy { account_ids: Some(vec![acc_a.clone(), work.id.clone()]), ..Default::default() } }).await;
    let agent_id = made["agent"]["id"].as_str().unwrap().to_string();
    pass(&a).await;
    let st = pass(&b).await;
    assert_eq!(st.offered_accounts.iter().map(|o| o.config.email()).collect::<Vec<_>>(), vec!["work@x.test"]);
    assert_eq!(b.store().get_agent(&agent_id).unwrap().unwrap().policy.account_ids, Some(vec![acc_b.clone()]));

    // Edited on B, which doesn't have the work account: A keeps it.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut pol = b.store().get_agent(&agent_id).unwrap().unwrap().policy;
    pol.allow_attachments = true;
    user(&b, Command::UpdateAgent { agent_id: agent_id.clone(), name: None, policy: Some(pol) }).await;
    pass(&b).await;
    pass(&a).await;
    let there = a.store().get_agent(&agent_id).unwrap().unwrap().policy;
    assert!(there.allow_attachments);
    let mut ids = there.account_ids.unwrap();
    ids.sort();
    let mut want = vec![acc_a, work.id];
    want.sort();
    assert_eq!(ids, want);

    // Once B adds the account, the agent has it there too.
    let work_b: Account = serde_json::from_value(
        user(&b, Command::AddAccount { config: AccountConfig::Gmail { email: "work@x.test".into(), display_name: "Work".into() }, credential: Credential::Password { password: "x".into() } }).await,
    )
    .unwrap();
    let st = pass(&b).await;
    assert!(st.offered_accounts.is_empty());
    let mut ids = b.store().get_agent(&agent_id).unwrap().unwrap().policy.account_ids.unwrap();
    ids.sort();
    let mut want = vec![acc_b, work_b.id];
    want.sort();
    assert_eq!(ids, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn smart_folder_order_follows() {
    let (a, _fa, _, b, _fb, _, _dirs) = paired().await;
    let mut ids = vec![];
    for name in ["One", "Two", "Three"] {
        let v = user(&a, Command::SaveSmartFolder { folder: SmartFolder { name: name.into(), query: name.to_lowercase(), ..Default::default() } }).await;
        ids.push(v["id"].as_str().map(String::from).unwrap_or_else(|| v.as_str().unwrap().to_string()));
    }
    pass(&a).await;
    pass(&b).await;
    let names = |e: &Arc<Engine>| e.store().list_smart_folders().unwrap().into_iter().map(|f| f.name).collect::<Vec<_>>();
    assert_eq!(names(&b), ["One", "Two", "Three"]);
    tokio::time::sleep(Duration::from_millis(5)).await;
    user(&a, Command::ReorderSmartFolders { ids: vec![ids[2].clone(), ids[0].clone(), ids[1].clone()] }).await;
    pass(&a).await;
    pass(&b).await;
    assert_eq!(names(&b), ["Three", "One", "Two"]);
    pass(&b).await;
    pass(&a).await;
    assert_eq!(names(&a), ["Three", "One", "Two"]);
}
