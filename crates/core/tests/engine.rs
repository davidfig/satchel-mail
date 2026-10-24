//! Engine-level tests with an in-process fake provider: agent permissions,
//! proposals (warnings, de-duplication, staleness), the op queue, and spam rules.

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
    /// Mail the "server" delivers on the next sync: (remote id, raw, remote mailbox).
    incoming: Mutex<Vec<(String, Vec<u8>, String)>>,
    /// Events the server puts on the calendar on the next sync: (remote id, ics).
    events: Mutex<Vec<(String, String)>>,
    /// `notify` of each calendar delete pushed to the server.
    cal_deletes: Mutex<Vec<bool>>,
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
            RemoteMailbox { remote_id: "Junk".into(), name: "Junk".into(), path: "Junk".into(), role: MailboxRole::Junk, is_label: false },
        ])?;
        let incoming: Vec<_> = self.incoming.lock().unwrap().drain(..).collect();
        for (rid, raw, mb) in incoming {
            ctx.put_raw_message(&rid, None, &raw, Flags::default(), &[(mb, None)], None)?;
        }
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

struct SharedCal(Arc<Fake>);
#[async_trait]
impl CalendarProvider for SharedCal {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let map = ctx.set_calendars(&[RemoteCalendar { remote_id: "cal".into(), name: "Calendar".into(), color: None, read_only: false }])?;
        let events: Vec<_> = self.0.events.lock().unwrap().drain(..).collect();
        for (rid, ics) in events {
            ctx.put_event(&map["cal"], &rid, None, &ics)?;
        }
        Ok(())
    }
    async fn apply(&self, _ctx: &SyncCtx, op: &CalendarOp) -> Result<ApplyOutcome> {
        if let CalendarOp::Delete { notify, .. } = op {
            self.0.cal_deletes.lock().unwrap().push(*notify);
        }
        Ok(ApplyOutcome::default())
    }
}

impl ProviderFactory for FakeFactory {
    fn create(&self, _account: &Account, _creds: CredentialHandle) -> Result<Providers> {
        Ok(Providers { mail: Some(Arc::new(Shared(self.0.clone()))), calendar: Some(Arc::new(SharedCal(self.0.clone()))) })
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

fn deliver(fake: &Fake, rid: &str, raw: Vec<u8>, mailbox: &str) {
    fake.incoming.lock().unwrap().push((rid.into(), raw, mailbox.into()));
}

fn in_role(engine: &Engine, account: &str, rid: &str, role: MailboxRole) -> bool {
    let Some(id) = engine.store().message_id_by_remote(account, rid).unwrap() else { return false };
    let mb = engine.store().mailbox_by_role(account, role).unwrap().unwrap();
    engine.store().memberships(&id).unwrap().iter().any(|(m, _)| *m == mb.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_rules_outrank_the_server_filter() {
    let (engine, fake, account, _dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();

    // Agents can't decide what's spam.
    let a = agent(&engine, AgentPolicy::default()).await;
    let err = engine.handle(&a, Command::MarkSpam { message_ids: vec![invoice.clone()] }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);

    let r = engine.handle(&Caller::User, Command::MarkSpam { message_ids: vec![invoice.clone()] }).await.unwrap();
    assert_eq!(r["senders"], json!(["bill@evil.test"]));
    assert!(in_role(&engine, &account, "r3", MailboxRole::Junk));

    // New mail from the sender lands in the inbox; the rule moves it.
    deliver(&fake, "r4", raw("m4", "Vendor <Bill@Evil.test>", "Invoice again", None), "INBOX");
    // The server puts mail from a real person in Junk.
    deliver(&fake, "r5", raw("m5", "Carol <carol@x.test>", "Lunch?", None), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "r4", MailboxRole::Junk)).await;
    assert!(in_role(&engine, &account, "r5", MailboxRole::Junk), "no rule for Carol yet");

    // Not spam moves her back, and her later mail never stays in Junk.
    let lunch = engine.store().message_id_by_remote(&account, "r5").unwrap().unwrap();
    engine.handle(&Caller::User, Command::MarkNotSpam { message_ids: vec![lunch] }).await.unwrap();
    assert!(in_role(&engine, &account, "r5", MailboxRole::Inbox));
    deliver(&fake, "r6", raw("m6", "Carol <carol@x.test>", "Lunch, take two", None), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "r6", MailboxRole::Inbox)).await;

    let rules: Vec<SenderRule> = serde_json::from_value(engine.handle(&Caller::User, Command::ListSenderRules).await.unwrap()).unwrap();
    assert_eq!(rules.len(), 2);
    // Removing a rule lets the server's filter decide again.
    engine.handle(&Caller::User, Command::SetSenderRule { email: "bill@evil.test".into(), verdict: None }).await.unwrap();
    deliver(&fake, "r7", raw("m7", "Vendor <bill@evil.test>", "Third time", None), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "r7").ok().flatten().is_some()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(in_role(&engine, &account, "r7", MailboxRole::Inbox));
}

#[tokio::test(flavor = "multi_thread")]
async fn warns_about_impersonation_and_deceptive_links() {
    let (engine, fake, account, _dir) = setup().await;
    // The user has written to Bob, so "Bob" from another address is suspicious.
    deliver(&fake, "s1", raw("s1", "Me <me@x.test>", "Re: Kickoff", Some("m1")).replace_to("Bob <bob@x.test>"), "INBOX");
    let mut spoof = raw("s2", "Bob <bob.office@freemail.test>", "Quick favor", None);
    spoof = String::from_utf8(spoof).unwrap().replacen("\r\n\r\n", "\r\nAuthentication-Results: mx.x.test; spf=fail; dkim=none; dmarc=fail\r\n\r\n", 1).into_bytes();
    deliver(&fake, "s2", spoof, "INBOX");
    let html = "From: Bank <alerts@bank-secure.test>\r\nTo: me@x.test\r\nSubject: Verify\r\nMessage-ID: <s3@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\
        List-Unsubscribe: <https://bank-secure.test/u>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\nContent-Type: text/html\r\n\r\n\
        <p><a href=\"https://bank-secure.test/login\">https://www.bank.com/login</a></p>\r\n";
    deliver(&fake, "s3", html.as_bytes().to_vec(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "s3").ok().flatten().is_some()).await;

    let get = |rid: &str| {
        let id = engine.store().message_id_by_remote(&account, rid).unwrap().unwrap();
        let engine = engine.clone();
        async move { serde_json::from_value::<MessageDetail>(engine.handle(&Caller::User, Command::GetMessage { message_id: id }).await.unwrap()).unwrap() }
    };
    let m = get("s2").await;
    let kinds: Vec<WarningKind> = m.warnings.iter().map(|w| w.kind).collect();
    assert_eq!(kinds, vec![WarningKind::Impersonation, WarningKind::AuthFailed], "{:?}", m.warnings);
    assert!(m.warnings[0].message.contains("bob@x.test"));

    let m = get("s3").await;
    assert_eq!(m.warnings.len(), 1);
    assert_eq!(m.warnings[0].kind, WarningKind::DeceptiveLink);
    assert!(m.unsubscribe.as_ref().is_some_and(|u| u.one_click));

    // Bob himself gets no warnings.
    assert!(get("r1").await.warnings.is_empty());
}

trait ReplaceTo {
    fn replace_to(self, to: &str) -> Vec<u8>;
}

impl ReplaceTo for Vec<u8> {
    fn replace_to(self, to: &str) -> Vec<u8> {
        String::from_utf8(self).unwrap().replacen("To: me@x.test", &format!("To: {to}"), 1).into_bytes()
    }
}

fn invite_ics(uid: &str, my_status: &str) -> String {
    [
        "BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Test//EN", "BEGIN:VEVENT", &format!("UID:{uid}"), "DTSTAMP:20261020T100000Z",
        "DTSTART:20261101T150000Z", "DTEND:20261101T153000Z", "SUMMARY:Claim your prize", "ORGANIZER:mailto:noreply@prize.test",
        &format!("ATTENDEE;PARTSTAT={my_status};RSVP=TRUE:mailto:me@x.test"), "END:VEVENT", "END:VCALENDAR", "",
    ]
    .join("\r\n")
}

fn invite_mail(id: &str, ics: &str) -> Vec<u8> {
    format!(
        "From: Prize <noreply@prize.test>\r\nTo: me@x.test\r\nSubject: Invitation {id}\r\nMessage-ID: <{id}@prize.test>\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nYou won.\r\n\
         --b\r\nContent-Type: text/calendar; method=REQUEST\r\nContent-Disposition: attachment; filename=invite.ics\r\n\r\n{ics}\r\n--b--\r\n",
        chrono::Utc::now().to_rfc2822()
    )
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
async fn marking_spam_silently_removes_its_invitation() {
    let (engine, fake, account, _dir) = setup().await;
    // The server auto-added the unanswered invite; the user accepted the other one.
    let unanswered = invite_ics("spam-1", "NEEDS-ACTION");
    let accepted = invite_ics("spam-2", "ACCEPTED");
    deliver(&fake, "i1", invite_mail("i1", &unanswered), "INBOX");
    deliver(&fake, "i2", invite_mail("i2", &accepted), "INBOX");
    fake.events.lock().unwrap().extend([("e1".to_string(), unanswered), ("e2".to_string(), accepted)]);
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().events_by_uid(&account, "spam-2").map(|v| !v.is_empty()).unwrap_or(false)
        && engine.store().message_id_by_remote(&account, "i2").ok().flatten().is_some()).await;

    let ids: Vec<String> = ["i1", "i2"].iter().map(|r| engine.store().message_id_by_remote(&account, r).unwrap().unwrap()).collect();
    let r = engine.handle(&Caller::User, Command::MarkSpam { message_ids: ids }).await.unwrap();
    assert_eq!(r["invites_removed"], json!(1));
    assert!(engine.store().events_by_uid(&account, "spam-1").unwrap().is_empty());
    assert_eq!(engine.store().events_by_uid(&account, "spam-2").unwrap().len(), 1, "answered invitations stay");
    wait_for(|| !fake.cal_deletes.lock().unwrap().is_empty()).await;
    assert_eq!(*fake.cal_deletes.lock().unwrap(), vec![false], "the organizer is never notified");

    // An invite the server adds later, from the now-blocked sender, comes off after the next sync.
    let later = invite_ics("spam-3", "NEEDS-ACTION");
    deliver(&fake, "i3", invite_mail("i3", &later), "INBOX");
    fake.events.lock().unwrap().push(("e3".into(), later));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| fake.cal_deletes.lock().unwrap().len() == 2).await;
    assert!(engine.store().events_by_uid(&account, "spam-3").unwrap().is_empty());
    assert!(in_role(&engine, &account, "i3", MailboxRole::Junk));
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

/// Drop a file into the AI folder's inbox and ingest it; returns the processed copy.
async fn drop_proposal(engine: &Arc<Engine>, dir: &tempfile::TempDir, name: &str, text: &str) -> String {
    let inbox = dir.path().join("mirror/proposals/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(name), text).unwrap();
    engine.ingest_dropped_proposals().await;
    assert!(!inbox.join(name).exists(), "ingested files leave the inbox");
    std::fs::read_to_string(dir.path().join("mirror/proposals/processed").join(name)).unwrap()
}

fn field<'a>(file: &'a str, key: &str) -> Option<&'a str> {
    file.lines().find_map(|l| l.strip_prefix(&format!("{key}: ")))
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_proposal_files_need_no_token_and_report_back() {
    let (engine, _fake, account, dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let invoice_thread = engine.store().thread_of(&invoice).unwrap().unwrap();
    let kickoff = engine.store().message_id_by_remote(&account, "r1").unwrap().unwrap();
    let kickoff_thread = engine.store().thread_of(&kickoff).unwrap().unwrap();

    // A token-less archive comes from the built-in AI folder agent.
    let done = drop_proposal(&engine, &dir, "archive.md", &format!("---\naction: archive\nthreads: [{invoice_thread}]\nwhy: Paid already\nagent: Claude\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("pending"), "{done}");
    let id = field(&done, "proposal").unwrap().to_string();
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: id.clone() }).await.unwrap()).unwrap();
    assert_eq!(p.agent_name, "AI folder");
    assert_eq!(p.rationale, "[Claude] Paid already");
    assert!(matches!(&p.action, ProposalAction::Archive { message_ids } if message_ids == &vec![invoice.clone()]));
    let agents: Vec<Agent> = serde_json::from_value(engine.handle(&Caller::User, Command::ListAgents).await.unwrap()).unwrap();
    assert!(agents.iter().any(|a| a.builtin && a.name == "AI folder"));

    // The user's decision and note are written back to the file.
    engine.handle(&Caller::User, Command::RejectProposal { proposal_id: id, note: Some("Not \"yet\"".into()) }).await.unwrap();
    let done = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/archive.md")).unwrap();
    assert_eq!(field(&done, "status"), Some("rejected"), "{done}");
    assert_eq!(field(&done, "note"), Some(r#""Not \"yet\"""#));

    // A reply goes to the sender of the thread's last message, with the body as text.
    let done = drop_proposal(&engine, &dir, "reply.md", &format!("---\naction: reply\nthread: {kickoff_thread}\nwhy: Confirm\n---\nMonday works.\n")).await;
    let id = field(&done, "proposal").unwrap().to_string();
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: id.clone() }).await.unwrap()).unwrap();
    let ProposalAction::Draft { draft } = &p.action else { panic!("expected a draft") };
    assert_eq!(draft.to, vec![Address { name: Some("Carol".into()), email: "carol@x.test".into() }]);
    assert_eq!(draft.subject, "Re: Kickoff");
    assert_eq!(draft.body_text, "Monday works.");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: id, edited: None }).await.unwrap();
    let done = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/reply.md")).unwrap();
    assert_eq!(field(&done, "status"), Some("approved"));

    // Mistakes come back as errors, under a free name if the old one is taken.
    let done = drop_proposal(&engine, &dir, "oops.md", "---\naction: shred\n---\n").await;
    assert_eq!(field(&done, "status"), Some("error"));
    assert!(field(&done, "error").unwrap().contains("unknown action"));
    std::fs::write(dir.path().join("mirror/proposals/inbox/oops.md"), "no header").unwrap();
    engine.ingest_dropped_proposals().await;
    let again = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/oops-2.md")).unwrap();
    assert!(again.contains("no header"));
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_proposals_can_be_turned_off_but_tokens_still_work() {
    let (engine, _fake, account, dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let mut s = engine.settings();
    s.mirror.accept_proposals = false;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();

    let done = drop_proposal(&engine, &dir, "flag.md", &format!("---\naction: flag\nmessages: [{invoice}]\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("error"));
    assert!(field(&done, "error").unwrap().contains("turned off"));

    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "Codex".into(), policy: AgentPolicy::default() }).await.unwrap();
    let token = v["token"].as_str().unwrap();
    let done = drop_proposal(&engine, &dir, "flag2.md", &format!("---\naction: flag\nmessages: [{invoice}]\nagent_token: {token}\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("pending"), "{done}");
    assert!(!done.contains(token), "the token is stripped");
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: field(&done, "proposal").unwrap().into() }).await.unwrap()).unwrap();
    assert_eq!(p.agent_name, "Codex");
}
