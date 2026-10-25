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
    let status: Proposal = serde_json::from_value(engine.handle(&a, Command::GetProposal { proposal_id: p1.id }).await.unwrap()).unwrap();
    assert_eq!(status.status, ProposalStatus::Applied);
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
    assert_eq!(err.code, ErrorCode::Forbidden);

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
