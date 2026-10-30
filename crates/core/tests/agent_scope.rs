//! Proposals stay inside the agent's scope: accounts, dates, attachments,
//! mailboxes and events; and agents can never run a write command directly.

mod support;

use satchel_core::blobs::MemBlobStore;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::{MessageRef, Store};
use satchel_core::{EngineOptions, Mode};
use support::mail::*;

/// Adds a second account (the fake server gives it the same three messages).
async fn second_account(engine: &Arc<Engine>) -> String {
    let acc: Account = serde_json::from_value(
        engine
            .handle(&Caller::User, Command::AddAccount { config: AccountConfig::Gmail { email: "other@y.test".into(), display_name: "Other".into() }, credential: Credential::Password { password: "x".into() } })
            .await
            .unwrap(),
    )
    .unwrap();
    wait_for(|| engine.store().message_id_by_remote(&acc.id, "r3").ok().flatten().is_some()).await;
    acc.id
}

/// A message from Bob dated now, with a PDF attached.
fn with_attachment(id: &str) -> Vec<u8> {
    format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: Contract\r\nMessage-ID: <{id}@x.test>\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
         --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=contract.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n",
        chrono::Utc::now().to_rfc2822()
    )
    .into_bytes()
}

async fn propose(engine: &Arc<Engine>, a: &Caller, action: ProposalAction, threads: Vec<String>) -> Result<Proposal, ApiError> {
    engine.handle(a, Command::CreateProposal { action, rationale: "r".into(), source_thread_ids: threads }).await.map(|v| serde_json::from_value(v).unwrap())
}

fn fwd_draft(account: &str, forward_of: &str) -> Draft {
    Draft { account_id: account.into(), to: vec![Address::new("bob@x.test")], subject: "Fwd".into(), forward_of: Some(forward_of.into()), ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarding_stays_within_the_agents_scope() {
    let (engine, fake, account, _dir) = setup().await;
    let other = second_account(&engine).await;
    let foreign = engine.store().message_id_by_remote(&other, "r3").unwrap().unwrap();

    // A message in an account the agent can't see.
    let one = agent(&engine, AgentPolicy { account_ids: Some(vec![account.clone()]), allow_attachments: true, ..Default::default() }).await;
    let err = propose(&engine, &one, ProposalAction::Draft { draft: fwd_draft(&account, &foreign) }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // A message the agent can see, but in another account than the draft.
    let all = agent(&engine, AgentPolicy { allow_attachments: true, ..Default::default() }).await;
    let err = propose(&engine, &all, ProposalAction::Draft { draft: fwd_draft(&account, &foreign) }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput, "{err:?}");

    // A message older than the agent's date range (the fixtures are dated 2026-10-20).
    let recent = agent(&engine, AgentPolicy { since_days: Some(1), allow_attachments: true, ..Default::default() }).await;
    let old = engine.store().message_id_by_remote(&account, "r1").unwrap().unwrap();
    let err = propose(&engine, &recent, ProposalAction::Draft { draft: fwd_draft(&account, &old) }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // Attachments need allow_attachments, whether forwarded or added.
    deliver(&fake, "att", with_attachment("att"), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    // Whichever account syncs first gets it.
    let find = |acc: &str| engine.store().message_id_by_remote(acc, "att").unwrap().map(|m| (m, acc.to_string()));
    wait_for(|| find(&account).or(find(&other)).is_some()).await;
    let (contract, account) = find(&account).or(find(&other)).unwrap();
    let no_att = agent(&engine, AgentPolicy::default()).await;
    let err = propose(&engine, &no_att, ProposalAction::Draft { draft: fwd_draft(&account, &contract) }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let added = Draft {
        attachments: vec![OutgoingAttachment { filename: "a.txt".into(), mime_type: "text/plain".into(), data_base64: "aGk=".into() }],
        forward_of: None,
        ..fwd_draft(&account, &contract)
    };
    let err = propose(&engine, &no_att, ProposalAction::Draft { draft: added }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);

    // Allowed: the forwarded message's thread counts as the conversation, and
    // its version is recorded, so the proposal goes stale when it changes.
    let p = propose(&engine, &all, ProposalAction::Draft { draft: fwd_draft(&account, &contract) }, vec![]).await.unwrap();
    assert!(!p.warnings.iter().any(|w| w.contains("bob@x.test")), "{:?}", p.warnings);
    engine.handle(&Caller::User, Command::SetFlags { message_ids: vec![contract.clone()], seen: None, flagged: Some(true) }).await.unwrap();
    let err = engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: p.id, edited: None }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_only_forwards_attachments_from_its_own_account() {
    let (engine, fake, account, _dir) = setup().await;
    let other = second_account(&engine).await;
    deliver(&fake, "att", with_attachment("att"), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    // Whichever account syncs first gets it; the draft is in the other one.
    let find = |acc: &str| engine.store().message_id_by_remote(acc, "att").unwrap();
    wait_for(|| find(&account).or(find(&other)).is_some()).await;
    let (foreign, draft_account) = match find(&other) {
        Some(m) => (m, account.clone()),
        None => (find(&account).unwrap(), other.clone()),
    };
    let err = engine.handle(&Caller::User, Command::Send { draft: fwd_draft(&draft_account, &foreign) }).await.unwrap_err();
    assert!(err.message.contains("another account"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn event_proposals_show_attendee_changes_and_keep_their_uid() {
    let (engine, _fake, _account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let cal = engine.store().list_calendars(None).unwrap().remove(0).cal.id;
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let new = Event {
        id: None,
        calendar_id: cal.clone(),
        uid: None,
        title: "Standup".into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + chrono::Duration::minutes(15),
        all_day: false,
        timezone: None,
        rrule: None,
        attendees: vec![Attendee { email: "alice@x.test".into(), name: None, status: PartStat::NeedsAction, optional: false }],
        organizer: None,
        status: EventStatus::Confirmed,
        my_status: None,
    };
    let stored: Event = serde_json::from_value(engine.handle(&Caller::User, Command::CreateEvent { event: new.clone() }).await.unwrap()).unwrap();

    // Swapping an attendee says who is invited and who is removed.
    let mut changed = stored.clone();
    changed.attendees[0].email = "eve@evil.test".into();
    let p = propose(&engine, &a, ProposalAction::UpdateEvent { event: changed.clone() }, vec![]).await.unwrap();
    assert!(p.warnings.iter().any(|w| w == "Sends invitations to eve@evil.test"), "{:?}", p.warnings);
    assert!(p.warnings.iter().any(|w| w == "Removes alice@x.test"), "{:?}", p.warnings);

    // A new organizer is called out.
    let mut org = stored.clone();
    org.organizer = Some(Address::new("boss@x.test"));
    let p = propose(&engine, &a, ProposalAction::UpdateEvent { event: org }, vec![]).await.unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("organizer") && w.contains("boss@x.test")), "{:?}", p.warnings);

    // An unchanged event has no warnings.
    let mut same = stored.clone();
    same.title = "Standup (moved)".into();
    let p = propose(&engine, &a, ProposalAction::UpdateEvent { event: same }, vec![]).await.unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);

    // The UID can't change.
    let mut uid = stored.clone();
    uid.uid = Some("someone-elses-meeting".into());
    let err = propose(&engine, &a, ProposalAction::UpdateEvent { event: uid }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // A new event never keeps the agent's UID or organizer.
    let mut forged = new.clone();
    forged.uid = Some(stored.uid.clone().unwrap());
    forged.organizer = Some(Address::new("boss@x.test"));
    let p = propose(&engine, &a, ProposalAction::CreateEvent { event: forged }, vec![]).await.unwrap();
    let ProposalAction::CreateEvent { event } = &p.action else { panic!() };
    assert_eq!(event.uid, None);
    assert_eq!(event.organizer, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn proposed_mailboxes_belong_to_the_messages_account() {
    let (engine, _fake, account, _dir) = setup().await;
    let other = second_account(&engine).await;
    let msg = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let foreign_archive = engine.store().mailbox_by_role(&other, MailboxRole::Archive).unwrap().unwrap().id;
    let own_archive = engine.store().mailbox_by_role(&account, MailboxRole::Archive).unwrap().unwrap().id;

    // A mailbox in an account the agent can't see.
    let one = agent(&engine, AgentPolicy { account_ids: Some(vec![account.clone()]), ..Default::default() }).await;
    let err = propose(&engine, &one, ProposalAction::Move { message_ids: vec![msg.clone()], mailbox_id: foreign_archive.clone() }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = propose(&engine, &one, ProposalAction::Labels { message_ids: vec![msg.clone()], add: vec![foreign_archive.clone()], remove: vec![] }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // A mailbox the agent can see, but in another account than the messages.
    let all = agent(&engine, AgentPolicy::default()).await;
    let err = propose(&engine, &all, ProposalAction::Move { message_ids: vec![msg.clone()], mailbox_id: foreign_archive.clone() }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = propose(&engine, &all, ProposalAction::Labels { message_ids: vec![msg.clone()], add: vec![], remove: vec![foreign_archive] }, vec![]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // The messages' own account is fine.
    propose(&engine, &one, ProposalAction::Move { message_ids: vec![msg], mailbox_id: own_archive }, vec![]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn source_threads_respect_the_date_range() {
    let (engine, _fake, account, _dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let thread = engine.store().thread_of(&invoice).unwrap().unwrap();
    let draft = Draft { account_id: account.clone(), to: vec![Address::new("bill@evil.test")], subject: "Re".into(), ..Default::default() };
    // The fixtures are dated 2026-10-20, outside a one-day range.
    let recent = agent(&engine, AgentPolicy { since_days: Some(1), ..Default::default() }).await;
    let err = propose(&engine, &recent, ProposalAction::Draft { draft: draft.clone() }, vec![thread.clone()]).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let all = agent(&engine, AgentPolicy { since_days: None, ..Default::default() }).await;
    propose(&engine, &all, ProposalAction::Draft { draft }, vec![thread]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn info_hides_the_data_dir_from_agents() {
    let (engine, _fake, _account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let info = engine.handle(&a, Command::Info).await.unwrap();
    assert!(info.get("data_dir").is_none(), "{info}");
    assert!(info.get("mirror_root").is_some());
    let info = engine.handle(&Caller::User, Command::Info).await.unwrap();
    assert!(info.get("data_dir").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_errors_only_name_shared_accounts() {
    let (engine, _fake, account, dir) = setup().await;
    second_account(&engine).await;
    let third: Account = serde_json::from_value(
        engine
            .handle(&Caller::User, Command::AddAccount { config: AccountConfig::Gmail { email: "third@z.test".into(), display_name: "Third".into() }, credential: Credential::Password { password: "x".into() } })
            .await
            .unwrap(),
    )
    .unwrap();
    let mut s = engine.settings();
    s.mirror.account_ids = Some(vec![account.clone(), third.id.clone()]);
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let done = drop_proposal(&engine, &dir, "new.md", "---\naction: draft\nto: bob@x.test\nsubject: Hi\n---\nHello\n").await;
    let error = field(&done, "error").unwrap();
    assert!(error.contains("missing `account:`"), "{error}");
    assert!(error.contains("me@x.test") && error.contains("third@z.test"), "{error}");
    assert!(!error.contains("other@y.test"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_can_never_run_write_commands() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let msg = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let ids = vec![msg.clone()];
    let mb = engine.store().mailbox_by_role(&account, MailboxRole::Archive).unwrap().unwrap().id;
    let cal = engine.store().list_calendars(None).unwrap().remove(0).cal.id;
    let start = chrono::Utc::now();
    let event = Event {
        id: None,
        calendar_id: cal.clone(),
        uid: None,
        title: "x".into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start,
        all_day: false,
        timezone: None,
        rrule: None,
        attendees: vec![],
        organizer: None,
        status: EventStatus::Confirmed,
        my_status: None,
    };
    let writes = vec![
        Command::Send { draft: send_draft(&account) },
        Command::SaveDraft { draft: send_draft(&account) },
        Command::DeleteDraft { draft_id: "d".into() },
        Command::Trash { message_ids: ids.clone() },
        Command::Archive { message_ids: ids.clone() },
        Command::Move { message_ids: ids.clone(), mailbox_id: mb.clone() },
        Command::SetFlags { message_ids: ids.clone(), seen: Some(true), flagged: None },
        Command::SetLabels { message_ids: ids.clone(), add: vec![mb.clone()], remove: vec![] },
        Command::MarkAllRead { mailbox_id: Some(mb.clone()), role: None },
        Command::EmptyMailbox { mailbox_id: None, role: Some(MailboxRole::Junk) },
        Command::MarkSpam { message_ids: ids.clone() },
        Command::CreateEvent { event: event.clone() },
        Command::UpdateEvent { event },
        Command::DeleteEvent { event_id: "e".into() },
        Command::SetCalendarVisible { calendar_id: cal, visible: false },
        Command::RespondToInvite { event_id: "e".into(), response: PartStat::Accepted },
        Command::Unsubscribe { message_id: msg },
        Command::DeleteContact { contact_id: "c".into() },
        Command::UpdateSettings { settings: engine.settings() },
        Command::GetSettings,
        Command::ListOutbox,
        Command::ListDrafts,
        Command::RemoveAccount { account_id: account.clone() },
        Command::SyncNow { account_id: None },
    ];
    for cmd in writes {
        let name = format!("{cmd:?}").split([' ', '{']).next().unwrap().to_string();
        let err = engine.handle(&a, cmd).await.expect_err(&name);
        assert_eq!(err.code, ErrorCode::Forbidden, "{name}");
    }
    assert!(engine.store().list_outbox().unwrap().is_empty(), "nothing was queued");
}

/// A Gmail-style fake, where mailboxes are labels.
struct Labels(Arc<Fake>);
#[async_trait::async_trait]
impl MailProvider for Labels {
    async fn test(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn sync(&self, ctx: &SyncCtx) -> anyhow::Result<()> {
        self.0.sync(ctx).await
    }
    async fn fetch_raw(&self, ctx: &SyncCtx, msg: &MessageRef) -> anyhow::Result<Vec<u8>> {
        self.0.fetch_raw(ctx, msg).await
    }
    async fn apply(&self, ctx: &SyncCtx, op: &ResolvedOp) -> anyhow::Result<ApplyOutcome> {
        self.0.apply(ctx, op).await
    }
    async fn wait_for_changes(&self, ctx: &SyncCtx, max: Duration) -> anyhow::Result<bool> {
        self.0.wait_for_changes(ctx, max).await
    }
    fn uses_labels(&self) -> bool {
        true
    }
}

struct LabelsFactory(Arc<Fake>);
impl ProviderFactory for LabelsFactory {
    fn create(&self, _account: &Account, _creds: CredentialHandle) -> anyhow::Result<Providers> {
        Ok(Providers { mail: Some(Arc::new(Labels(self.0.clone()))), calendar: None, contacts: None })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_from_another_account_are_refused_for_the_user_too() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let engine = Engine::with_parts(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MemBlobStore::default()),
        EngineOptions { data_dir: dir.path().to_path_buf(), mode: Mode::Full, secrets: Arc::new(MemSecretStore::default()), factory: Arc::new(LabelsFactory(fake)), start_workers: false },
    )
    .unwrap();
    let mut s = engine.settings();
    s.mirror.root = Some(dir.path().join("mirror").display().to_string());
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let add = |email: &str| Command::AddAccount { config: AccountConfig::Gmail { email: email.into(), display_name: "x".into() }, credential: Credential::Password { password: "x".into() } };
    let a: Account = serde_json::from_value(engine.handle(&Caller::User, add("me@x.test")).await.unwrap()).unwrap();
    let b: Account = serde_json::from_value(engine.handle(&Caller::User, add("other@y.test")).await.unwrap()).unwrap();
    wait_for(|| [&a.id, &b.id].iter().all(|acc| engine.store().message_id_by_remote(acc, "r3").ok().flatten().is_some())).await;
    let msg = engine.store().message_id_by_remote(&a.id, "r3").unwrap().unwrap();
    let foreign = engine.store().mailbox_by_role(&b.id, MailboxRole::Archive).unwrap().unwrap().id;
    let own = engine.store().mailbox_by_role(&a.id, MailboxRole::Archive).unwrap().unwrap().id;

    for (add, remove) in [(vec![foreign.clone()], vec![]), (vec![], vec![foreign.clone()])] {
        let err = engine.handle(&Caller::User, Command::SetLabels { message_ids: vec![msg.clone()], add, remove }).await.unwrap_err();
        assert!(err.message.contains("another account"), "{err:?}");
    }
    let err = engine.handle(&Caller::User, Command::Move { message_ids: vec![msg.clone()], mailbox_id: foreign }).await.unwrap_err();
    assert!(err.message.contains("between accounts"), "{err:?}");
    engine.handle(&Caller::User, Command::SetLabels { message_ids: vec![msg], add: vec![own], remove: vec![] }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_out_of_scope_event_looks_like_a_missing_one() {
    let (engine, _fake, account, _dir) = setup().await;
    let other = second_account(&engine).await;
    let cal_of = |acc: &str| engine.store().list_calendars(None).unwrap().into_iter().find(|c| c.cal.account_id == acc).map(|c| c.cal.id);
    wait_for(|| cal_of(&other).is_some()).await;
    let foreign_cal = cal_of(&other).unwrap();
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let event = Event {
        id: None,
        calendar_id: foreign_cal.clone(),
        uid: None,
        title: "Board meeting".into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        timezone: None,
        rrule: None,
        attendees: vec![],
        organizer: None,
        status: EventStatus::Confirmed,
        my_status: None,
    };
    let stored: Event = serde_json::from_value(engine.handle(&Caller::User, Command::CreateEvent { event: event.clone() }).await.unwrap()).unwrap();
    let foreign = stored.id.clone().unwrap();
    let a = agent(&engine, AgentPolicy { account_ids: Some(vec![account]), ..Default::default() }).await;
    let same = |x: ApiError, y: ApiError| {
        assert_eq!((x.code, &x.message), (y.code, &y.message));
        assert_eq!(x.code, ErrorCode::NotFound);
    };

    // Reading it.
    let get = |id: &str| engine.handle(&a, Command::GetEvent { event_id: id.into() });
    same(get(&foreign).await.unwrap_err(), get("no-such-event").await.unwrap_err());

    // Proposing to change, cancel or answer it.
    for action in [
        |id: String, e: &Event| ProposalAction::UpdateEvent { event: Event { id: Some(id), ..e.clone() } },
        |id: String, _: &Event| ProposalAction::CancelEvent { event_id: id },
        |id: String, _: &Event| ProposalAction::RespondToInvite { event_id: id, response: PartStat::Accepted },
    ] {
        let real = propose(&engine, &a, action(foreign.clone(), &stored), vec![]).await.unwrap_err();
        let missing = propose(&engine, &a, action("no-such-event".into(), &stored), vec![]).await.unwrap_err();
        same(real, missing);
    }

    // Adding one to its calendar.
    let real = propose(&engine, &a, ProposalAction::CreateEvent { event: event.clone() }, vec![]).await.unwrap_err();
    let missing = propose(&engine, &a, ProposalAction::CreateEvent { event: Event { calendar_id: "no-such-calendar".into(), ..event } }, vec![]).await.unwrap_err();
    same(real, missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_lists_show_agents_only_messages_inside_the_date_limit() {
    let (engine, fake, account, _dir) = setup().await;
    // A reply today, filed in Archive, to the ten-day-old kickoff thread in INBOX.
    let reply = format!(
        "From: Dave <dave@x.test>\r\nTo: me@x.test\r\nSubject: Re: Kickoff\r\nMessage-ID: <m4@x.test>\r\nIn-Reply-To: <m2@x.test>\r\nReferences: <m1@x.test> <m2@x.test>\r\nDate: {}\r\n\r\nFresh reply\r\n",
        chrono::Utc::now().to_rfc2822()
    );
    deliver(&fake, "r4", reply.into_bytes(), "Archive");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "r4").ok().flatten().is_some()).await;
    let a = agent(&engine, AgentPolicy { since_days: Some(5), ..Default::default() }).await;
    let list = |query: ThreadQuery| {
        let engine = engine.clone();
        let a = a.clone();
        async move { inbox_threads(engine.handle(&a, Command::ListThreads { query }).await.unwrap()) }
    };

    // Only the kickoff thread, and only as its reply shows it; no invoice.
    let all = list(ThreadQuery::default()).await;
    assert_eq!(all.len(), 1, "{all:?}");
    let t = &all[0];
    assert_eq!((t.message_count, t.subject.as_str()), (1, "Re: Kickoff"));
    assert_eq!(t.participants.iter().map(|p| p.email.as_str()).collect::<Vec<_>>(), ["dave@x.test"]);
    assert!(t.snippet.contains("Fresh reply"), "{}", t.snippet);
    let inbox = engine.store().mailbox_by_role(&account, MailboxRole::Inbox).unwrap().unwrap().id;
    assert!(!t.mailbox_ids.contains(&inbox));

    // Its old messages don't put it in the inbox, or match a search.
    assert!(list(ThreadQuery { role: Some(MailboxRole::Inbox), ..Default::default() }).await.is_empty());
    assert!(list(ThreadQuery { mailbox_id: Some(inbox), ..Default::default() }).await.is_empty());
    assert!(list(ThreadQuery { search: Some("from:bob".into()), ..Default::default() }).await.is_empty());
    assert_eq!(list(ThreadQuery { search: Some("from:dave".into()), ..Default::default() }).await.len(), 1);
}
