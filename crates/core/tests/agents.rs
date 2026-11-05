//! Agents read within their policy and can only propose; proposed drafts carry warnings and go stale.

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn agents_read_and_propose_but_cannot_act() {
    let (engine, fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;

    let threads = inbox_threads(engine.handle(&a, Command::ListThreads { query: ThreadQuery { role: Some(MailboxRole::Inbox), ..Default::default() } }).await.unwrap());
    assert_eq!(threads.len(), 2, "kickoff + reply thread together, invoice separate");
    let kickoff = threads.iter().find(|t| t.message_count == 2).unwrap();

    // Direct writes are refused for agents.
    let ids = engine.store().thread_message_ids(&kickoff.thread_id).unwrap();
    let err = engine.handle(&a, Command::Archive { message_ids: ids.clone() }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let err = engine.handle(&a, Command::Send { draft: Draft { account_id: account.clone(), to: vec![Address::new("x@y")], ..Default::default() } }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);

    // Proposing works and is de-duplicated.
    let p1: Proposal = serde_json::from_value(
        engine.handle(&a, Command::CreateProposal { action: ProposalAction::Archive { message_ids: ids.clone() }, rationale: "done".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    let p2: Proposal = serde_json::from_value(
        engine.handle(&a, Command::CreateProposal { action: ProposalAction::Archive { message_ids: ids.clone() }, rationale: "again".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    assert_eq!(p1.id, p2.id);

    // Agents can't approve their own proposals.
    let err = engine.handle(&a, Command::ApproveProposal { proposal_id: p1.id.clone(), edited: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);

    // The user approves: messages move locally and the op reaches the provider.
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p1.id.clone(), edited: None }).await.unwrap();
    let archive = engine.store().mailbox_by_role(&account, MailboxRole::Archive).unwrap().unwrap();
    for id in &ids {
        let mbs = engine.store().memberships(id).unwrap();
        assert_eq!(mbs.len(), 1);
        assert_eq!(mbs[0].0, archive.id);
    }
    wait_for(|| fake.applied.lock().unwrap().iter().any(|x| x == "move")).await;
    let status: Proposal = serde_json::from_value(engine.handle(&a, Command::GetProposal { proposal_id: p1.id.clone() }).await.unwrap()).unwrap();
    assert_eq!(status.status, ProposalStatus::Applied);

    // A decided proposal can't be decided again.
    let err = engine.handle(&Caller::User, Command::RejectProposal { proposal_id: p1.id.clone(), note: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p1.id, edited: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_limits_what_agents_see() {
    let (engine, _fake, _account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy { account_ids: Some(vec!["someone-else".into()]), ..Default::default() }).await;
    let threads = inbox_threads(engine.handle(&a, Command::ListThreads { query: Default::default() }).await.unwrap());
    assert!(threads.is_empty());
    let accounts: Vec<Account> = serde_json::from_value(engine.handle(&a, Command::ListAccounts).await.unwrap()).unwrap();
    assert!(accounts.is_empty());
    let any = engine.store().list_threads(&satchel_core::store::ThreadFilter { limit: 1, ..Default::default() }).unwrap();
    let err = engine.handle(&a, Command::GetThread { thread_id: any[0].thread_id.clone() }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    let no_cal = agent(&engine, AgentPolicy { allow_calendar: false, ..Default::default() }).await;
    let err = engine.handle(&no_cal, Command::ListCalendars).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_warnings_and_staleness() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let thread = engine.store().thread_of(&invoice).unwrap().unwrap();

    let draft = Draft {
        account_id: account.clone(),
        to: vec![Address::new("bill@evil.test"), Address::new("attacker@elsewhere.test")],
        subject: "Fwd".into(),
        body_text: "here you go".into(),
        ..Default::default()
    };
    let p: Proposal = serde_json::from_value(
        engine.handle(&a, Command::CreateProposal { action: ProposalAction::Draft { draft }, rationale: "asked to".into(), source_thread_ids: vec![thread] }).await.unwrap(),
    )
    .unwrap();
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(p.warnings[0].contains("attacker@elsewhere.test"));

    // Approving a draft creates a local draft, never a send.
    let r = engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id, edited: None }).await.unwrap();
    let draft_id = r["draft_id"].as_str().unwrap();
    assert!(engine.store().get_draft(draft_id).unwrap().is_some());
    assert!(engine.store().list_outbox().unwrap().iter().all(|o| o.kind != "send"));

    // A proposal becomes stale when its target changes before approval.
    let flag: Proposal = serde_json::from_value(
        engine.handle(&a, Command::CreateProposal { action: ProposalAction::Flag { message_ids: vec![invoice.clone()], flagged: true }, rationale: "".into(), source_thread_ids: vec![] }).await.unwrap(),
    )
    .unwrap();
    engine.handle(&Caller::User, Command::SetFlags { message_ids: vec![invoice.clone()], seen: Some(true), flagged: None }).await.unwrap();
    let err = engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: flag.id.clone(), edited: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let again: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: flag.id.clone() }).await.unwrap()).unwrap();
    assert_eq!(again.status, ProposalStatus::Stale);
    // After re-review the user can still approve it.
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: flag.id, edited: None }).await.unwrap();
    let audit: Vec<AuditEntry> = serde_json::from_value(engine.handle(&Caller::User, Command::ListAudit { limit: Some(50) }).await.unwrap()).unwrap();
    assert!(audit.iter().any(|e| e.action == "proposal_approved"));
    let _ = json!(null);
}

async fn propose(engine: &Arc<Engine>, a: &Caller, action: ProposalAction) -> Result<Proposal, ApiError> {
    engine.handle(a, Command::CreateProposal { action, rationale: "r".into(), source_thread_ids: vec![] }).await.map(|v| serde_json::from_value(v).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_on_special_mailboxes_warn() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let msg = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let role = |r| engine.store().mailbox_by_role(&account, r).unwrap().unwrap().id;
    // On this folder account, adding one mailbox is a move to it.
    let p = propose(&engine, &a, ProposalAction::Labels { message_ids: vec![msg.clone()], add: vec![role(MailboxRole::Trash)], remove: vec![] }).await.unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("adds Trash")), "{:?}", p.warnings);
    assert!(p.warnings.iter().any(|w| w.contains("Moves")), "{:?}", p.warnings);
    let p = propose(&engine, &a, ProposalAction::Labels { message_ids: vec![msg.clone()], add: vec![], remove: vec![role(MailboxRole::Junk), role(MailboxRole::Archive)] }).await.unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("removes Junk") && w.contains("removes Archive")), "{:?}", p.warnings);
    let p = propose(&engine, &a, ProposalAction::Labels { message_ids: vec![msg], add: vec![], remove: vec![role(MailboxRole::Inbox)] }).await.unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_drafts_are_plain_text_with_clean_attachment_types() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy { allow_attachments: true, ..Default::default() }).await;
    let draft = Draft { account_id: account.clone(), to: vec![Address::new("bob@x.test")], subject: "Hi".into(), body_text: "Hello".into(), body_html: Some("<p>Not what Review shows</p>".into()), ..Default::default() };
    let p = propose(&engine, &a, ProposalAction::Draft { draft: draft.clone() }).await.unwrap();
    let ProposalAction::Draft { draft: kept } = &p.action else { panic!() };
    assert_eq!(kept.body_html, None);
    for bad in ["text/plain\r\nContent-Disposition: inline", "text/plain; name=x", "text", "a/b/c", "text/ plain"] {
        let mut d = draft.clone();
        d.attachments = vec![OutgoingAttachment { filename: "a.txt".into(), mime_type: bad.into(), data_base64: "aGk=".into() }];
        let err = propose(&engine, &a, ProposalAction::Draft { draft: d }).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
    }
    let mut d = draft;
    d.attachments = vec![OutgoingAttachment { filename: "a.txt".into(), mime_type: "application/vnd.ms-excel".into(), data_base64: "aGk=".into() }];
    propose(&engine, &a, ProposalAction::Draft { draft: d }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn approvals_record_the_agent_id() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let Caller::Agent(agent_row) = &a else { panic!() };
    let msg = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let p = propose(&engine, &a, ProposalAction::Flag { message_ids: vec![msg], flagged: true }).await.unwrap();
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id, edited: None }).await.unwrap();
    let audit: Vec<AuditEntry> = serde_json::from_value(engine.handle(&Caller::User, Command::ListAudit { limit: Some(50) }).await.unwrap()).unwrap();
    let e = audit.iter().find(|e| e.action == "proposal_approved").unwrap();
    assert_eq!(e.detail["agent_id"], json!(agent_row.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn text_attachments_need_allow_attachments() {
    let (engine, fake, account, _dir) = setup().await;
    let mail = format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: Notes\r\nMessage-ID: <notes@x.test>\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
         --b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=secret.txt\r\n\r\nthe secret\r\n--b--\r\n",
        chrono::Utc::now().to_rfc2822()
    );
    deliver(&fake, "notes", mail.into_bytes(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "notes").ok().flatten().is_some()).await;
    let id = engine.store().message_id_by_remote(&account, "notes").unwrap().unwrap();
    let att = engine.store().message_row(&id).unwrap().unwrap().attachments.into_iter().find(|a| !a.inline).unwrap();
    let get = Command::GetAttachment { message_id: id.clone(), attachment_id: att.id.clone() };
    let no = agent(&engine, AgentPolicy { allow_attachments: false, ..Default::default() }).await;
    assert_eq!(engine.handle(&no, get.clone()).await.unwrap_err().code, ErrorCode::Forbidden);
    let yes = agent(&engine, AgentPolicy { allow_attachments: true, ..Default::default() }).await;
    let v = engine.handle(&yes, get).await.unwrap();
    assert!(v["text"].as_str().unwrap().contains("the secret"), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn free_time_is_bounded() {
    let (engine, _fake, _account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let from = chrono::Utc::now();
    let ask = |days| Command::FindFreeTime { from, to: from + chrono::Duration::days(days), duration_minutes: 5, day_start_hour: Some(0), day_end_hour: Some(24), timezone: None };
    assert_eq!(engine.handle(&a, ask(5000)).await.unwrap_err().code, ErrorCode::InvalidInput);
    let slots: Vec<FreeSlot> = serde_json::from_value(engine.handle(&a, ask(400)).await.unwrap()).unwrap();
    assert!(!slots.is_empty() && slots.len() <= 500, "{}", slots.len());
}

/// A `satchel call` beside the running app (agent mode) can't change what
/// agents may do: settings and agent management need the app's process.
#[tokio::test(flavor = "multi_thread")]
async fn agent_mode_cannot_change_settings_or_agents() {
    use satchel_core::{EngineOptions, Mode};
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::with_parts(
        Arc::new(satchel_core::store::Store::open_in_memory().unwrap()),
        Arc::new(satchel_core::blobs::MemBlobStore::default()),
        EngineOptions {
            data_dir: dir.path().to_path_buf(),
            mode: Mode::Agent,
            secrets: Arc::new(satchel_core::secrets::MemSecretStore::default()),
            factory: Arc::new(FakeFactory(Arc::new(Fake::default()))),
            start_workers: false,
        },
    )
    .unwrap();
    let cmds = vec![
        Command::UpdateSettings { settings: engine.settings() },
        Command::CreateAgent { name: "x".into(), policy: AgentPolicy::default() },
        Command::UpdateAgent { agent_id: "a".into(), name: None, policy: Some(AgentPolicy::default()) },
        Command::RevokeAgent { agent_id: "a".into() },
        Command::SetCalendarVisible { calendar_id: "c".into(), visible: false },
        Command::RemoveAccount { account_id: "a".into() },
        Command::RejectProposal { proposal_id: "p".into(), note: None },
        Command::RetryOutbox { id: "o".into() },
    ];
    for cmd in cmds {
        let name = format!("{cmd:?}").split([' ', '{']).next().unwrap().to_string();
        let err = engine.handle(&Caller::User, cmd).await.expect_err(&name);
        assert_eq!(err.code, ErrorCode::Conflict, "{name}");
    }
}
