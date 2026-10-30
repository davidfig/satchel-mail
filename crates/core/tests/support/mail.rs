//! A fake mail and calendar server, an engine set up against it, and helpers.

pub use std::sync::Arc;
pub use std::time::Duration;

pub use satchel_api::*;
pub use satchel_core::provider::*;
pub use satchel_core::{Caller, Engine};
pub use serde_json::{Value, json};
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use satchel_core::blobs::MemBlobStore;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::{MessageRef, Store};
use satchel_core::{EngineOptions, Mode};

#[derive(Default)]
pub struct Fake {
    pub applied: Mutex<Vec<String>>,
    /// Mail the "server" delivers on the next sync: (remote id, raw, remote mailbox).
    pub incoming: Mutex<Vec<(String, Vec<u8>, String)>>,
    /// Events the server puts on the calendar on the next sync: (remote id, ics).
    pub events: Mutex<Vec<(String, String)>>,
    /// `notify` of each calendar delete pushed to the server.
    pub cal_deletes: Mutex<Vec<bool>>,
    /// Folders on the "server": (remote id, name, path). Remote ids follow the
    /// path, like IMAP, so renames change them.
    pub folders: Mutex<Vec<(String, String, String)>>,
    /// Remote mailbox of each move pushed to the server.
    pub moves: Mutex<Vec<String>>,
    /// Remote ids of the messages each move acted on.
    pub moved_ids: Mutex<Vec<Vec<String>>>,
    /// Give moved messages a new remote id, like IMAP.
    pub rekey_moves: std::sync::atomic::AtomicBool,
    /// Raw messages handed to the server to send.
    pub sent: Mutex<Vec<Vec<u8>>>,
    /// Make sends fail as if the connection dropped before the server confirmed.
    pub sends_unconfirmed: std::sync::atomic::AtomicBool,
    /// Full raw messages by remote id, for `fetch_raw` (delivered ones).
    pub originals: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    /// Copies without attachments by remote id, for `fetch_without_attachments`.
    pub slim: Mutex<std::collections::HashMap<String, satchel_mime::Slim>>,
    /// How many times `fetch_raw` ran.
    pub raw_fetches: std::sync::atomic::AtomicUsize,
    /// Queries sent to the place service.
    pub place_queries: Mutex<Vec<String>>,
}

pub fn raw(id: &str, from: &str, subject: &str, irt: Option<&str>) -> Vec<u8> {
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
            RemoteMailbox { remote_id: "Sent".into(), name: "Sent".into(), path: "Sent".into(), role: MailboxRole::Sent, is_label: false },
            RemoteMailbox { remote_id: "Junk".into(), name: "Junk".into(), path: "Junk".into(), role: MailboxRole::Junk, is_label: false },
            RemoteMailbox { remote_id: "Trash".into(), name: "Trash".into(), path: "Trash".into(), role: MailboxRole::Trash, is_label: false },
        ]
        .into_iter()
        .chain(self.folders.lock().unwrap().iter().map(|(id, name, path)| RemoteMailbox {
            remote_id: id.clone(),
            name: name.clone(),
            path: path.clone(),
            role: MailboxRole::Custom,
            is_label: false,
        }))
        .collect::<Vec<_>>())?;
        let incoming: Vec<_> = self.incoming.lock().unwrap().drain(..).collect();
        for (rid, raw, mb) in incoming {
            self.originals.lock().unwrap().insert(rid.clone(), raw.clone());
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
    async fn fetch_raw(&self, _ctx: &SyncCtx, msg: &MessageRef) -> Result<Vec<u8>> {
        self.raw_fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.originals.lock().unwrap().get(&msg.remote_id).cloned().ok_or_else(|| anyhow::anyhow!("not on the server"))
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
            ResolvedOp::CreateMailbox { .. } => "mailbox_create",
            ResolvedOp::UpdateMailbox { .. } => "mailbox_update",
            ResolvedOp::DeleteMailbox { .. } => "mailbox_delete",
        };
        self.applied.lock().unwrap().push(name.into());
        if let ResolvedOp::Send { raw, .. } = op {
            self.sent.lock().unwrap().push(raw.clone());
            if self.sends_unconfirmed.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(MaybeSent(anyhow::anyhow!("connection reset")).into());
            }
        }
        let mut out = ApplyOutcome::default();
        let mut folders = self.folders.lock().unwrap();
        match op {
            ResolvedOp::Move { messages, to_remote_mailbox } => {
                self.moves.lock().unwrap().push(to_remote_mailbox.clone());
                self.moved_ids.lock().unwrap().push(messages.iter().map(|m| m.remote_id.clone()).collect());
                // Like IMAP: a moved message gets a new identity in its new folder.
                for m in messages.iter().filter(|_| self.rekey_moves.load(std::sync::atomic::Ordering::Relaxed)) {
                    out.rekeyed.push((m.id.clone(), format!("{to_remote_mailbox}#{}", m.id), vec![(to_remote_mailbox.clone(), Some(1))]));
                }
            }
            ResolvedOp::CreateMailbox { name, path, .. } => {
                let id = format!("F-{path}");
                folders.push((id.clone(), name.clone(), path.clone()));
                out.mailbox_remote_id = Some(id);
            }
            ResolvedOp::UpdateMailbox { old_path, name, path, .. } => {
                for f in folders.iter_mut() {
                    let new_path = if f.2 == *old_path {
                        f.1 = name.clone();
                        path.clone()
                    } else if let Some(rest) = f.2.strip_prefix(&format!("{old_path}/")) {
                        format!("{path}/{rest}")
                    } else {
                        continue;
                    };
                    let new_id = format!("F-{new_path}");
                    out.mailbox_renamed.push((f.0.clone(), new_id.clone()));
                    *f = (new_id, f.1.clone(), new_path);
                }
            }
            ResolvedOp::DeleteMailbox { remote_id, .. } => folders.retain(|f| f.0 != *remote_id),
            _ => {}
        }
        Ok(out)
    }
    async fn wait_for_changes(&self, _ctx: &SyncCtx, _max: Duration) -> Result<bool> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(true)
    }
}

pub struct FakeFactory(pub Arc<Fake>);

pub struct Shared(pub Arc<Fake>);
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
    async fn fetch_without_attachments(&self, ctx: &SyncCtx, msg: &MessageRef) -> Result<Download> {
        let slim = self.0.slim.lock().unwrap().get(&msg.remote_id).cloned();
        match slim {
            Some(s) => Ok(Download::Slim(s)),
            None => Ok(Download::Full(self.0.fetch_raw(ctx, msg).await?)),
        }
    }
    async fn apply(&self, ctx: &SyncCtx, op: &ResolvedOp) -> Result<ApplyOutcome> {
        self.0.apply(ctx, op).await
    }
    async fn wait_for_changes(&self, ctx: &SyncCtx, max: Duration) -> Result<bool> {
        self.0.wait_for_changes(ctx, max).await
    }
}

pub struct SharedCal(pub Arc<Fake>);
#[async_trait]
impl CalendarProvider for SharedCal {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        let map = ctx.set_calendars(&[
            RemoteCalendar { remote_id: "cal".into(), name: "Calendar".into(), color: None, read_only: false },
            RemoteCalendar { remote_id: "personal".into(), name: "Personal".into(), color: None, read_only: false },
        ])?;
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
        Ok(Providers { mail: Some(Arc::new(Shared(self.0.clone()))), calendar: Some(Arc::new(SharedCal(self.0.clone()))), contacts: None })
    }
    fn places(&self) -> Option<Arc<dyn PlaceSearch>> {
        Some(Arc::new(FakePlaces(self.0.clone())))
    }
}

/// A place service that finds "<query> Cafe".
pub struct FakePlaces(pub Arc<Fake>);
#[async_trait]
impl PlaceSearch for FakePlaces {
    fn name(&self) -> &'static str {
        "Fake Maps"
    }
    async fn suggest(&self, query: &str) -> Result<Vec<PlaceSuggestion>> {
        self.0.place_queries.lock().unwrap().push(query.to_string());
        let name = format!("{query} Cafe");
        Ok(vec![PlaceSuggestion { text: format!("{name}, 1 Main St"), label: name, detail: Some("1 Main St".into()), source: PlaceSource::OpenStreetMap }])
    }
}

pub async fn setup() -> (Arc<Engine>, Arc<Fake>, String, tempfile::TempDir) {
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

pub async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not met in time");
}

pub async fn agent(engine: &Arc<Engine>, policy: AgentPolicy) -> Caller {
    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "Test".into(), policy }).await.unwrap();
    let token = v["token"].as_str().unwrap().to_string();
    Caller::Agent(engine.authenticate_agent(&token).unwrap())
}

pub fn inbox_threads(v: Value) -> Vec<ThreadSummary> {
    serde_json::from_value(v).unwrap()
}

pub fn deliver(fake: &Fake, rid: &str, raw: Vec<u8>, mailbox: &str) {
    fake.incoming.lock().unwrap().push((rid.into(), raw, mailbox.into()));
}

pub fn in_role(engine: &Engine, account: &str, rid: &str, role: MailboxRole) -> bool {
    let Some(id) = engine.store().message_id_by_remote(account, rid).unwrap() else { return false };
    let mb = engine.store().mailbox_by_role(account, role).unwrap().unwrap();
    engine.store().memberships(&id).unwrap().iter().any(|(m, _)| *m == mb.id)
}


pub trait ReplaceTo {
    fn replace_to(self, to: &str) -> Vec<u8>;
}

impl ReplaceTo for Vec<u8> {
    fn replace_to(self, to: &str) -> Vec<u8> {
        String::from_utf8(self).unwrap().replacen("To: me@x.test", &format!("To: {to}"), 1).into_bytes()
    }
}

pub fn invite_ics(uid: &str, my_status: &str) -> String {
    [
        "BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Test//EN", "BEGIN:VEVENT", &format!("UID:{uid}"), "DTSTAMP:20261020T100000Z",
        "DTSTART:20261101T150000Z", "DTEND:20261101T153000Z", "SUMMARY:Claim your prize", "ORGANIZER:mailto:noreply@prize.test",
        &format!("ATTENDEE;PARTSTAT={my_status};RSVP=TRUE:mailto:me@x.test"), "END:VEVENT", "END:VCALENDAR", "",
    ]
    .join("\r\n")
}

pub fn invite_mail(id: &str, ics: &str) -> Vec<u8> {
    format!(
        "From: Prize <noreply@prize.test>\r\nTo: me@x.test\r\nSubject: Invitation {id}\r\nMessage-ID: <{id}@prize.test>\r\nDate: {}\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nYou won.\r\n\
         --b\r\nContent-Type: text/calendar; method=REQUEST\r\nContent-Disposition: attachment; filename=invite.ics\r\n\r\n{ics}\r\n--b--\r\n",
        chrono::Utc::now().to_rfc2822()
    )
    .into_bytes()
}

/// Drop a file into the AI folder's inbox and ingest it; returns the processed copy.
pub async fn drop_proposal(engine: &Arc<Engine>, dir: &tempfile::TempDir, name: &str, text: &str) -> String {
    let inbox = dir.path().join("mirror/proposals/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(name), text).unwrap();
    engine.ingest_dropped_proposals().await;
    assert!(!inbox.join(name).exists(), "ingested files leave the inbox");
    std::fs::read_to_string(dir.path().join("mirror/proposals/processed").join(name)).unwrap()
}

pub fn field<'a>(file: &'a str, key: &str) -> Option<&'a str> {
    file.lines().find_map(|l| l.strip_prefix(&format!("{key}: ")))
}

pub fn folder(engine: &Arc<Engine>, path: &str) -> Option<Mailbox> {
    engine.store().list_mailboxes(None).unwrap().into_iter().find(|m| m.path == path)
}

pub fn send_draft(account: &str) -> Draft {
    Draft {
        account_id: account.into(),
        to: vec![Address::new("bob@x.test")],
        bcc: vec![Address::new("secret@x.test")],
        subject: "Hello".into(),
        body_text: "Hi Bob".into(),
        ..Default::default()
    }
}
