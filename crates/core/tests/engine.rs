//! Engine-level tests with an in-process fake provider: agent permissions,
//! proposals (warnings, de-duplication, staleness), and the op queue.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use satchel_api::*;
use satchel_core::blobs::MemBlobStore;
use satchel_core::provider::*;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::{MessageRef, Store};
use satchel_core::{Caller, Engine, EngineOptions, Mode};
use serde_json::{Value, json};

#[derive(Default)]
struct Fake {
    applied: Mutex<Vec<String>>,
}

fn raw(id: &str, from: &str, subject: &str, irt: Option<&str>) -> Vec<u8> {
    let mut s = format!("From: {from}\r\nTo: me@x.test\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n");
    if let Some(p) = irt {
        s.push_str(&format!("In-Reply-To: <{p}@x.test>\r\nReferences: <{p}@x.test>\r\n"));
    }
    s.push_str("\r\nBody of ");
    s.push_str(subject);
    s.push_str("\r\n");
    s.into_bytes()
}

#[async_trait]
impl MailProvider for Fake {
    async fn test(&self) -> Result<()> {
        Ok(())
    }
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        ctx.set_mailboxes(&[
            RemoteMailbox { remote_id: "INBOX".into(), name: "Inbox".into(), path: "INBOX".into(), role: MailboxRole::Inbox, is_label: false },
            RemoteMailbox { remote_id: "Archive".into(), name: "Archive".into(), path: "Archive".into(), role: MailboxRole::Archive, is_label: false },
        ])?;
        if ctx.message_by_remote("r1")?.is_none() {
            let f = Flags::default();
            ctx.put_raw_message("r1", None, &raw("m1", "Bob <bob@x.test>", "Kickoff", None), f.clone(), &[("INBOX".into(), Some(1))], None)?;
            ctx.put_raw_message("r2", None, &raw("m2", "Carol <carol@x.test>", "Re: Kickoff", Some("m1")), f.clone(), &[("INBOX".into(), Some(2))], None)?;
            ctx.put_raw_message("r3", None, &raw("m3", "Vendor <bill@evil.test>", "Invoice", None), f, &[("INBOX".into(), Some(3))], None)?;
        }
        Ok(())
    }
    async fn fetch_raw(&self, _ctx: &SyncCtx, _msg: &MessageRef) -> Result<Vec<u8>> {
        anyhow::bail!("not needed")
    }
    async fn apply(&self, _ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        let name = match op {
            ResolvedOp::SetFlags { .. } => "flags",
            ResolvedOp::Move { .. } => "move",
            ResolvedOp::Labels { .. } => "labels",
            ResolvedOp::Expunge { .. } => "expunge",
            ResolvedOp::SaveDraft { .. } => "save_draft",
            ResolvedOp::DeleteDraft { .. } => "delete_draft",
            ResolvedOp::Send { .. } => "send",
        };
        self.applied.lock().unwrap().push(name.into());
        Ok(ApplyOutcome::default())
    }
    async fn wait_for_changes(&self, _ctx: &SyncCtx, _max: Duration) -> Result<bool> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(true)
    }
}

struct FakeFactory(Arc<Fake>);

struct Shared(Arc<Fake>);
#[async_trait]
impl MailProvider for Shared {
    async fn test(&self) -> Result<()> {
        self.0.test().await
    }
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        self.0.sync(ctx).await
    }
    async fn fetch_raw(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        self.0.fetch_raw(ctx, msg).await
    }
    async fn apply(&self, ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        self.0.apply(ctx, op).await
    }
    async fn wait_for_changes(&self, ctx: &SyncCtx, max: Duration) -> Result<bool> {
        self.0.wait_for_changes(ctx, max).await
    }
}

impl ProviderFactory for FakeFactory {
    fn create(&self, _account: &Account, _creds: CredentialHandle) -> Result<Providers> {
        Ok(Providers { mail: Some(Arc::new(Shared(self.0.clone()))), calendar: None })
    }
}

async fn setup() -> (Arc<Engine>, Arc<Fake>, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let engine = Engine::with_parts(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MemBlobStore::default()),
        EngineOptions {
            data_dir: dir.path().to_path_buf(),
            mode: Mode::Full,
            secrets: Arc::new(MemSecretStore::default()),
            factory: Arc::new(FakeFactory(fake.clone())),
            start_workers: false,
        },
    )
    .unwrap();
    // Keep the AI mirror inside the temp dir.
    let mut s = engine.settings();
    s.mirror.root = Some(dir.path().join("mirror").display().to_string());
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let acc: Account = serde_json::from_value(
        engine
            .handle(&Caller::User, Command::AddAccount { config: AccountConfig::Gmail { email: "me@x.test".into(), display_name: "Me".into() }, credential: Credential::Password { password: "x".into() } })
            .await
            .unwrap(),
    )
    .unwrap();
    wait_for(|| engine.store().list_threads(&Default::default()).map(|_| ()).is_ok() && engine.store().message_id_by_remote(&acc.id, "r3").ok().flatten().is_some()).await;
    (engine, fake, acc.id, dir)
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not met in time");
}

async fn agent(engine: &Arc<Engine>, policy: AgentPolicy) -> Caller {
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "Test".into(), policy }).await.unwrap();
    let token = v["token"].as_str().unwrap().to_string();
    Caller::Agent(engine.authenticate_agent(&token).unwrap())
}

fn inbox_threads(v: Value) -> Vec<ThreadSummary> {
    serde_json::from_value(v).unwrap()
}

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
