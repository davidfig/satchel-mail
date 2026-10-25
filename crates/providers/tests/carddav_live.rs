//! Contacts sync against the local Stalwart test server (CardDAV), through
//! the whole engine. Needs `scripts/dev-servers start`; run with
//! `cargo test -p satchel-providers --test carddav_live -- --ignored`.

use std::sync::Arc;
use std::time::Duration;

use satchel_api::*;
use satchel_core::blobs::MemBlobStore;
use satchel_core::secrets::MemSecretStore;
use satchel_core::store::Store;
use satchel_core::{Caller, Engine, EngineOptions, Mode};

const DAV: &str = "http://127.0.0.1:8480";

async fn wait_for(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..240 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn get(http: &reqwest::Client, href: &str) -> Option<String> {
    let r = http.get(format!("{DAV}{href}")).basic_auth("alice", Some("test-pass-alice")).send().await.unwrap();
    r.status().is_success().then_some(r.text().await.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the local test servers (scripts/dev-servers start)"]
async fn carddav_round_trip_with_stalwart() {
    let dir = std::env::temp_dir().join(format!("satchel-carddav-live-{}", std::process::id()));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut settings = Settings::default();
    settings.mirror.enabled = false;
    store.set_setting("settings", &settings).unwrap();
    let engine = Engine::with_parts(
        store,
        Arc::new(MemBlobStore::default()),
        EngineOptions { data_dir: dir.clone(), mode: Mode::Full, secrets: Arc::new(MemSecretStore::default()), factory: Arc::new(satchel_providers::Factory::new()), start_workers: false },
    )
    .unwrap();
    let config = AccountConfig::Imap {
        email: "alice@satchel.test".into(),
        display_name: "Alice".into(),
        username: "alice".into(),
        imap_host: "127.0.0.1".into(),
        imap_port: 2993,
        imap_security: Security::Tls,
        smtp_host: "127.0.0.1".into(),
        smtp_port: 2465,
        smtp_security: Security::Tls,
        caldav_url: Some(format!("{DAV}/dav/cal")),
        carddav_url: None,
        allow_invalid_certs: true,
    };
    let acc: Account = serde_json::from_value(engine.handle(&Caller::User, Command::AddAccount { config, credential: Credential::Password { password: "test-pass-alice".into() } }).await.unwrap()).unwrap();
    // The address book is discovered from the CalDAV URL.
    wait_for("address book", async || !engine.store().list_address_books(Some(&acc.id)).unwrap().is_empty()).await;
    let book = engine.store().list_address_books(Some(&acc.id)).unwrap().remove(0);

    let tag = std::process::id();
    let mut c = Contact::default();
    c.fields.name = format!("Live Test {tag}");
    c.fields.emails = vec![ContactEmail { email: format!("live{tag}@satchel.test"), label: Some("work".into()) }];
    let c: Contact = serde_json::from_value(engine.handle(&Caller::User, Command::SaveContact { contact: c, address_book_id: Some(book.book.id.clone()) }).await.unwrap()).unwrap();
    let id = c.id.clone().unwrap();
    let card = async || engine.store().cards_of_contact(&id).unwrap().into_iter().next();
    wait_for("card pushed", async || card().await.is_some_and(|k| !k.remote_id.starts_with("local:"))).await;
    let href = card().await.unwrap().remote_id;
    let http = reqwest::Client::new();
    let on_server = get(&http, &href).await.expect("card on the server");
    assert!(on_server.contains(&format!("FN:Live Test {tag}")), "{on_server}");

    // Local edit goes up.
    let mut e = c.clone();
    e.fields.title = "Tester".into();
    engine.handle(&Caller::User, Command::SaveContact { contact: e, address_book_id: None }).await.unwrap();
    wait_for("edit pushed", async || get(&http, &href).await.is_some_and(|t| t.contains("TITLE:Tester"))).await;

    // A change made elsewhere comes down and merges.
    let text = get(&http, &href).await.unwrap();
    let mut f = satchel_ical::vcard::parse(&text).unwrap().fields;
    f.organization = "Elsewhere Inc".into();
    let changed = satchel_ical::vcard::build(Some(&text), "x", &f);
    let r = http.put(format!("{DAV}{href}")).basic_auth("alice", Some("test-pass-alice")).header("Content-Type", "text/vcard").body(changed).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    engine.handle(&Caller::User, Command::SyncNow { account_id: Some(acc.id.clone()) }).await.unwrap();
    wait_for("server change merged", async || engine.store().get_contact(&id).unwrap().is_some_and(|k| k.fields.organization == "Elsewhere Inc" && k.fields.title == "Tester")).await;

    // Deleting removes it from the server.
    engine.handle(&Caller::User, Command::DeleteContact { contact_id: id }).await.unwrap();
    wait_for("card deleted", async || get(&http, &href).await.is_none()).await;
    engine.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
