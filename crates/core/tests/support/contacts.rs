//! A fake address book server, an engine set up against it, and helpers.

pub use std::sync::Arc;
pub use std::time::Duration;

pub use satchel_api::*;
pub use satchel_core::provider::*;
pub use satchel_core::{Caller, Engine};
pub use serde_json::{Value, json};
use std::sync::Mutex;
use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use satchel_core::blobs::MemBlobStore;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::{MessageRef, Store};
use satchel_core::{EngineOptions, Mode};

pub const BOOK: &str = "/card/me/default/";

#[derive(Default)]
pub struct Server {
    /// Address book on the "server": href -> (etag, vCard).
    pub cards: Mutex<HashMap<String, (String, String)>>,
    /// Contact ops received: ("put" | "delete", href).
    pub ops: Mutex<Vec<(String, String)>>,
    pub etag_seq: Mutex<u32>,
    /// Act like a sign-in without the contacts scope.
    pub scope_missing: std::sync::atomic::AtomicBool,
}

impl Server {
    pub fn next_etag(&self) -> String {
        let mut n = self.etag_seq.lock().unwrap();
        *n += 1;
        format!("\"e{n}\"")
    }
    pub fn put(&self, href: &str, vcard: &str) {
        let etag = self.next_etag();
        self.cards.lock().unwrap().insert(href.into(), (etag, vcard.into()));
    }
    pub fn card(&self, href: &str) -> Option<ContactFields> {
        self.cards.lock().unwrap().get(href).and_then(|(_, v)| satchel_ical::vcard::parse(v)).map(|c| c.fields)
    }
}

pub fn mail(id: &str, from: &str, to: &str, subject: &str, irt: Option<&str>) -> Vec<u8> {
    let mut s = format!("From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n");
    if let Some(p) = irt {
        s.push_str(&format!("In-Reply-To: <{p}@x.test>\r\nReferences: <{p}@x.test>\r\n"));
    }
    s.push_str("\r\nHello\r\n");
    s.into_bytes()
}

pub struct FakeMail;
#[async_trait]
impl MailProvider for FakeMail {
    async fn test(&self) -> Result<()> {
        Ok(())
    }
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        ctx.set_mailboxes(&[
            RemoteMailbox { remote_id: "INBOX".into(), name: "Inbox".into(), path: "INBOX".into(), role: MailboxRole::Inbox, is_label: false },
            RemoteMailbox { remote_id: "Sent".into(), name: "Sent".into(), path: "Sent".into(), role: MailboxRole::Sent, is_label: false },
        ])?;
        if ctx.message_by_remote("s1")?.is_none() {
            let f = Flags { seen: true, ..Default::default() };
            let inbox = |uid| vec![("INBOX".to_string(), Some(uid))];
            // I wrote to Bob.
            ctx.put_raw_message("s1", None, &mail("s1", "Me <me@x.test>", "Bob Smith <bob@x.test>", "Plans", None), f.clone(), &[("Sent".into(), Some(1))], None)?;
            // Carol wrote once; never answered.
            ctx.put_raw_message("r1", None, &mail("r1", "Carol <carol@x.test>", "me@x.test", "Hi", None), f.clone(), &inbox(1), None)?;
            // Dave wrote, I replied in the thread.
            ctx.put_raw_message("r2", None, &mail("r2", "Dave Jones <dave@x.test>", "me@x.test", "Lunch", None), f.clone(), &inbox(2), None)?;
            ctx.put_raw_message("s2", None, &mail("s2", "Me <me@x.test>", "dave@x.test", "Re: Lunch", Some("r2")), f.clone(), &[("Sent".into(), Some(2))], None)?;
            // A notification I replied to anyway.
            ctx.put_raw_message("r3", None, &mail("r3", "Shop <no-reply@shop.test>", "me@x.test", "Order", None), f.clone(), &inbox(3), None)?;
            ctx.put_raw_message("s3", None, &mail("s3", "Me <me@x.test>", "no-reply@shop.test", "Re: Order", Some("r3")), f, &[("Sent".into(), Some(3))], None)?;
        }
        Ok(())
    }
    async fn fetch_raw(&self, _ctx: &SyncCtx, _msg: &MessageRef) -> Result<Vec<u8>> {
        anyhow::bail!("not needed")
    }
    async fn apply(&self, _ctx: &SyncCtx, _op: &ResolvedOp) -> Result<ApplyOutcome> {
        Ok(ApplyOutcome::default())
    }
    async fn wait_for_changes(&self, _ctx: &SyncCtx, _max: Duration) -> Result<bool> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(true)
    }
}

pub struct FakeContacts(pub Arc<Server>);
#[async_trait]
impl ContactsProvider for FakeContacts {
    async fn sync(&self, ctx: &SyncCtx) -> Result<()> {
        if self.0.scope_missing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow::Error::new(ContactsScopeMissing));
        }
        let map = ctx.set_address_books(&[RemoteAddressBook { remote_id: BOOK.into(), name: "Contacts".into(), read_only: false }])?;
        let local = &map[BOOK];
        let known = ctx.store.card_remote_ids(local)?;
        let cards = self.0.cards.lock().unwrap().clone();
        for (href, (etag, vcard)) in &cards {
            if known.get(href).is_none_or(|(_, e)| e.as_deref() != Some(etag.as_str())) {
                ctx.put_card(local, href, Some(etag), vcard)?;
            }
        }
        for gone in known.keys().filter(|k| !cards.contains_key(*k)) {
            ctx.delete_card_remote(local, gone)?;
        }
        Ok(())
    }
    async fn apply(&self, _ctx: &SyncCtx, op: &ContactOp) -> Result<ApplyOutcome> {
        let mut out = ApplyOutcome::default();
        match op {
            ContactOp::Put { card_remote_id, etag, uid, vcard, .. } => {
                let href = card_remote_id.clone().unwrap_or_else(|| format!("{BOOK}{uid}.vcf"));
                if card_remote_id.is_some() && self.0.cards.lock().unwrap().get(&href).map(|(e, _)| e.clone()) != *etag {
                    return Err(anyhow::Error::new(ApiError::new(ErrorCode::Conflict, "etag mismatch")));
                }
                self.0.put(&href, vcard);
                self.0.ops.lock().unwrap().push(("put".into(), href.clone()));
                out.card_etag = self.0.cards.lock().unwrap().get(&href).map(|(e, _)| e.clone());
                out.card_remote_id = Some(href);
            }
            ContactOp::Delete { card_remote_id, .. } => {
                self.0.cards.lock().unwrap().remove(card_remote_id);
                self.0.ops.lock().unwrap().push(("delete".into(), card_remote_id.clone()));
            }
        }
        Ok(out)
    }
    async fn fetch(&self, _ctx: &SyncCtx, _book: &str, href: &str) -> Result<Option<(Option<String>, String)>> {
        Ok(self.0.cards.lock().unwrap().get(href).map(|(e, v)| (Some(e.clone()), v.clone())))
    }
}

pub struct Factory(Arc<Server>, bool);
impl ProviderFactory for Factory {
    fn create(&self, _account: &Account, _creds: CredentialHandle) -> Result<Providers> {
        let contacts: Option<Arc<dyn ContactsProvider>> = if self.1 { Some(Arc::new(FakeContacts(self.0.clone()))) } else { None };
        Ok(Providers { mail: Some(Arc::new(FakeMail)), calendar: None, contacts })
    }
}

pub async fn setup(with_server: bool) -> (Arc<Engine>, Arc<Server>, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(Server::default());
    let engine = Engine::with_parts(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MemBlobStore::default()),
        EngineOptions {
            data_dir: dir.path().to_path_buf(),
            mode: Mode::Full,
            secrets: Arc::new(MemSecretStore::default()),
            factory: Arc::new(Factory(server.clone(), with_server)),
            start_workers: false,
        },
    )
    .unwrap();
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
    wait_for(|| engine.store().message_id_by_remote(&acc.id, "s3").ok().flatten().is_some()).await;
    (engine, server, acc.id, dir)
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

pub async fn contacts(engine: &Arc<Engine>, caller: &Caller) -> Vec<Contact> {
    serde_json::from_value(engine.handle(caller, Command::ListContacts { search: None, limit: None }).await.unwrap()).unwrap()
}

pub fn by_email<'a>(list: &'a [Contact], email: &str) -> Option<&'a Contact> {
    list.iter().find(|c| c.fields.emails.iter().any(|e| e.email.eq_ignore_ascii_case(email)))
}

pub async fn save(engine: &Arc<Engine>, c: Contact, book: Option<String>) -> Contact {
    serde_json::from_value(engine.handle(&Caller::User, Command::SaveContact { contact: c, address_book_id: book }).await.unwrap()).unwrap()
}


pub async fn sync_contacts(engine: &Arc<Engine>, account: &str) {
    engine.handle(&Caller::User, Command::SyncNow { account_id: Some(account.into()) }).await.unwrap();
}
