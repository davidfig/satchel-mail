//! Smart folders against the local Stalwart test server, through the whole
//! engine: real mail sent over SMTP, synced over IMAP, threaded and searched.
//! Needs `scripts/dev-servers start`; run with
//! `cargo test -p satchel-providers --test smart_folders_live -- --ignored`.
//!
//! Each run uses its own subject, so it doesn't care what else is in the
//! mailboxes. A passing run deletes its mail and any left by a failed run.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use satchel_api::*;
use satchel_core::blobs::MemBlobStore;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::Store;
use satchel_core::{Caller, Engine, EngineOptions, Mode};
use serde_json::Value;

const USER: Caller = Caller::User;
/// Starts the subject of all mail this test sends.
const PREFIX: &str = "smartlive";

/// A client signed in to one of the test accounts, as if on its own computer.
struct Client {
    engine: Arc<Engine>,
    account: Id,
    dir: PathBuf,
}

impl Client {
    async fn sign_in(user: &str, device: &str) -> Client {
        let dir = std::env::temp_dir().join(format!("satchel-smart-live-{}-{user}-{device}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let engine = Engine::with_parts(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MemBlobStore::default()),
            EngineOptions { data_dir: dir.clone(), mode: Mode::Full, secrets: Arc::new(MemSecretStore::default()), factory: Arc::new(satchel_providers::Factory::new()), start_workers: true },
        )
        .unwrap();
        let config = AccountConfig::Imap {
            email: format!("{user}@satchel.test"),
            display_name: user.into(),
            username: user.into(),
            imap_host: "127.0.0.1".into(),
            imap_port: 2993,
            imap_security: Security::Tls,
            smtp_host: "127.0.0.1".into(),
            smtp_port: 2465,
            smtp_security: Security::Tls,
            caldav_url: None,
            carddav_url: None,
            allow_invalid_certs: true,
        };
        let credential = Credential::Password { password: format!("test-pass-{user}") };
        let v = engine.handle(&USER, Command::AddAccount { config, credential }).await.unwrap();
        let account: Account = serde_json::from_value(v).unwrap();
        Client { engine, account: account.id, dir }
    }

    async fn run(&self, cmd: Command) -> Value {
        self.engine.handle(&USER, cmd).await.unwrap()
    }

    /// Keep syncing until `done`, as the app would when mail arrives.
    async fn sync_until(&self, what: &str, mut done: impl AsyncFnMut(&Client) -> bool) {
        for _ in 0..120 {
            if done(self).await {
                return;
            }
            self.run(Command::SyncNow { account_id: None }).await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn threads(&self, query: ThreadQuery) -> Vec<ThreadSummary> {
        serde_json::from_value(self.run(Command::ListThreads { query }).await).unwrap()
    }

    async fn search(&self, search: String) -> Vec<ThreadSummary> {
        self.threads(ThreadQuery { search: Some(search), ..Default::default() }).await
    }

    async fn in_folder(&self, folder: &SmartFolder) -> Vec<ThreadSummary> {
        self.threads(ThreadQuery { smart_folder_id: Some(folder.id.clone()), ..Default::default() }).await
    }

    async fn folder(&self, name: &str, query: String, badge: SmartBadge) -> SmartFolder {
        let folder = SmartFolder { name: name.into(), query, badge, ..Default::default() };
        serde_json::from_value(self.run(Command::SaveSmartFolder { folder }).await).unwrap()
    }

    /// What the sidebar shows for a folder.
    async fn badge(&self, id: &str) -> u32 {
        let folders: Vec<SmartFolder> = serde_json::from_value(self.run(Command::ListSmartFolders).await).unwrap();
        folders.into_iter().find(|f| f.id == id).expect("the folder is listed").count
    }

    async fn message_ids(&self, thread: &ThreadSummary) -> Vec<Id> {
        self.engine.store().thread_message_ids(&thread.thread_id).unwrap()
    }

    async fn send(&self, to: &str, subject: &str, body: &str, in_reply_to: Option<Id>) {
        let draft = Draft { account_id: self.account.clone(), to: vec![Address::new(to)], subject: subject.into(), body_text: body.into(), in_reply_to, ..Default::default() };
        self.run(Command::Send { draft }).await;
    }

    /// Delete all this test's mail for good, and wait for the server to hear.
    async fn remove_all(&self) {
        let query = format!("subject:{PREFIX} in:anywhere");
        self.run(Command::SyncNow { account_id: None }).await;
        // Once from where it is to Trash, once more from Trash.
        for _ in 0..2 {
            let mut ids = vec![];
            for t in self.search(query.clone()).await {
                ids.extend(self.message_ids(&t).await);
            }
            if !ids.is_empty() {
                self.run(Command::Trash { message_ids: ids }).await;
            }
            self.sync_until("changes to reach the server", async |c| {
                c.run(Command::ListOutbox).await.as_array().is_none_or(|ops| ops.is_empty())
            })
            .await;
        }
    }

    fn close(self) {
        self.engine.shutdown();
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the local test servers (scripts/dev-servers start)"]
async fn smart_folders_follow_real_mail() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let tag = format!("{PREFIX}{stamp}");
    let word = format!("zebra{stamp}");
    let (bob, carol) = (Client::sign_in("bob", "a").await, Client::sign_in("carol", "a").await);

    // Bob writes to Carol, and it arrives.
    bob.send("carol@satchel.test", &format!("Plan {tag}"), &format!("Lunch? {word}"), None).await;
    carol
        .sync_until("Bob's message", async |c| c.search(format!("subject:{tag}")).await.len() == 1)
        .await;

    let from_bob = carol.folder("From Bob", format!("subject:{tag} from:bob"), SmartBadge::Unread).await;
    let not_inbox = carol.folder("Not Inbox", format!("subject:{tag} -in:inbox"), SmartBadge::Total).await;
    let not_bob = carol.folder("Not Bob", format!("subject:{tag} -from:bob"), SmartBadge::Total).await;
    let sent = carol.folder("Sent", format!("subject:{tag} in:sent"), SmartBadge::Total).await;
    let read = carol.folder("Read", format!("subject:{tag} is:read"), SmartBadge::Total).await;
    let anywhere = carol.folder("Anywhere", format!("subject:{tag} in:anywhere"), SmartBadge::Total).await;
    let recent = carol.folder("Recent", format!("subject:{tag} newer_than:1d"), SmartBadge::Total).await;
    let old = carol.folder("Old", format!("subject:{tag} older_than:1d"), SmartBadge::Total).await;
    let words = carol.folder("Words", word.clone(), SmartBadge::Total).await;

    assert_eq!(carol.badge(&from_bob.id).await, 1, "one conversation with unread mail");
    assert_eq!(carol.badge(&recent.id).await, 1);
    assert_eq!(carol.badge(&old.id).await, 0);
    assert_eq!(carol.badge(&not_bob.id).await, 0);
    assert_eq!(carol.badge(&read.id).await, 0);
    let bobs = carol.in_folder(&from_bob).await;
    assert_eq!(bobs.len(), 1);
    assert!(bobs[0].subject.contains(&tag), "{}", bobs[0].subject);
    // The body is searchable once the indexer has been through it.
    carol.sync_until("the search index", async |c| c.badge(&words.id).await == 1).await;

    // Carol replies, so the conversation is now in her Inbox and her Sent.
    let first = carol.message_ids(&bobs[0]).await.remove(0);
    carol.send("bob@satchel.test", &format!("Re: Plan {tag}"), "Sounds good", Some(first)).await;
    carol.sync_until("the reply in Sent", async |c| c.badge(&sent.id).await == 1).await;
    let thread = carol.in_folder(&from_bob).await.remove(0);
    assert_eq!(thread.message_count, 2, "the reply joined Bob's conversation");

    // A reply elsewhere doesn't bring back what the folder leaves out.
    assert_eq!(carol.badge(&not_inbox.id).await, 0, "still in the Inbox");
    assert_eq!(carol.badge(&not_bob.id).await, 0, "Bob is still in it");
    assert_eq!(carol.badge(&from_bob.id).await, 1, "still unread");

    // The AI folder lists it, with a link that leads to the thread.
    carol.engine.mirror_pass(true).unwrap();
    let root = PathBuf::from(carol.engine.settings().mirror.root.unwrap());
    let md = std::fs::read_to_string(root.join("smart-folders.md")).unwrap();
    let section = md.split("## From Bob").nth(1).expect("the folder is listed");
    assert!(section.contains(&tag), "{md}");
    let link = section.split("](").nth(1).and_then(|l| l.split(')').next()).expect("a link").to_string();
    assert!(root.join(&link).is_file(), "{link}");

    // Carol reads it on another computer; the flag reaches this one through the server.
    let other = Client::sign_in("carol", "b").await;
    other.sync_until("the conversation on the other computer", async |c| c.search(format!("subject:{tag} in:sent")).await.len() == 1).await;
    let there = other.search(format!("subject:{tag}")).await.remove(0);
    let ids = other.message_ids(&there).await;
    other.run(Command::SetFlags { message_ids: ids, seen: Some(true), flagged: None }).await;
    carol.sync_until("the read flag", async |c| c.badge(&from_bob.id).await == 0).await;
    assert_eq!(carol.badge(&read.id).await, 1, "all of it is read now");
    assert_eq!(carol.in_folder(&from_bob).await.len(), 1, "the badge counts unread; the folder still lists it");
    other.close();

    // Mail in Trash leaves a folder unless the folder asks for it.
    let ids = carol.message_ids(&thread).await;
    carol.run(Command::Trash { message_ids: ids }).await;
    carol.sync_until("the move to Trash", async |c| c.in_folder(&from_bob).await.is_empty()).await;
    assert_eq!(carol.badge(&anywhere.id).await, 1);

    carol.remove_all().await;
    bob.remove_all().await;
    bob.close();
    carol.close();

    // Nothing is left on the server: a new client finds none of it.
    for user in ["bob", "carol"] {
        let fresh = Client::sign_in(user, "check").await;
        fresh
            .sync_until("a first sync", async |c| {
                let accounts: Vec<Account> = serde_json::from_value(c.run(Command::ListAccounts).await).unwrap();
                accounts.iter().all(|a| a.sync.last_success.is_some())
            })
            .await;
        assert!(fresh.search(format!("subject:{PREFIX} in:anywhere")).await.is_empty(), "{user} still has test mail");
        fresh.close();
    }
}
