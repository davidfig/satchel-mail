//! Stored message content: attachments only when asked for, and the
//! per-account storage limit (0036).

mod support;

use support::mail::*;

fn dated(id: &str, subject: &str, days_ago: i64, body: &str) -> Vec<u8> {
    let date = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc2822();
    format!("From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: {subject}\r\nMessage-ID: <{id}@x.test>\r\nDate: {date}\r\n\r\n{body}\r\n").into_bytes()
}

async fn sync(engine: &Arc<Engine>, account: &str, rid: &str) {
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(account, rid).ok().flatten().is_some()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_download_only_when_asked_for() {
    let (engine, fake, account, _dir) = setup().await;
    let pdf = "QUFB".repeat(50_000);
    let raw = format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: Report\r\nMessage-ID: <rep@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nThe report is attached.\r\n--b\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b--\r\n"
    );
    deliver(&fake, "rep", raw.into_bytes(), "INBOX");
    sync(&engine, &account, "rep").await;
    let id = engine.store().message_id_by_remote(&account, "rep").unwrap().unwrap();
    let row = engine.store().message_row(&id).unwrap().unwrap();
    assert!(row.partial, "stored without the PDF");
    assert!(engine.store().stored_bytes(&account).unwrap() < 10_000);

    let detail: MessageDetail = serde_json::from_value(engine.handle(&Caller::User, Command::GetMessage { message_id: id.clone() }).await.unwrap()).unwrap();
    assert_eq!(detail.text_body.trim(), "The report is attached.");
    let att = &detail.attachments[0];
    assert_eq!((att.filename.as_str(), att.size), ("report.pdf", 150_000), "size comes from the full message");

    let v = engine.handle(&Caller::User, Command::GetAttachment { message_id: id.clone(), attachment_id: att.id.clone() }).await.unwrap();
    assert_eq!(v["data_base64"].as_str().unwrap().len(), 200_000);
    assert!(!engine.store().message_row(&id).unwrap().unwrap().partial, "the full copy replaces the partial one");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_limit_keeps_the_newest_mail() {
    let (engine, fake, account, _dir) = setup().await;
    let mut s = engine.settings();
    s.storage_limit_mb = Some(1);
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let body = "x".repeat(400 * 1024);
    deliver(&fake, "old", dated("old", "Old", 30, &body), "INBOX");
    deliver(&fake, "mid", dated("mid", "Middle", 20, &body), "INBOX");
    deliver(&fake, "new", dated("new", "New", 1, &body), "INBOX");
    sync(&engine, &account, "new").await;
    let has_body = |rid: &str| {
        let id = engine.store().message_id_by_remote(&account, rid).unwrap().unwrap();
        engine.store().message_row(&id).unwrap().unwrap().blob_hash.is_some()
    };
    wait_for(|| !has_body("old")).await;
    assert!(has_body("new") && has_body("mid"));
    assert!(engine.store().stored_bytes(&account).unwrap() <= 1024 * 1024);

    // Opening freed mail downloads it again; the next sync frees the oldest.
    let old = engine.store().message_id_by_remote(&account, "old").unwrap().unwrap();
    let detail: MessageDetail = serde_json::from_value(engine.handle(&Caller::User, Command::GetMessage { message_id: old }).await.unwrap()).unwrap();
    assert!(detail.text_body.starts_with("xxx"));
    deliver(&fake, "newer", dated("newer", "Newer", 0, "short"), "INBOX");
    sync(&engine, &account, "newer").await;
    wait_for(|| !has_body("old")).await;
    assert!(has_body("newer") && has_body("new"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_stay_on_the_server_when_the_provider_can_leave_them_out() {
    let (engine, fake, account, _dir) = setup().await;
    let pdf = "QUFB".repeat(50_000);
    let raw = format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: Report\r\nMessage-ID: <rep@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nThe report is attached.\r\n--b\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b--\r\n"
    )
    .into_bytes();
    deliver(&fake, "rep", raw.clone(), "INBOX");
    sync(&engine, &account, "rep").await;
    let id = engine.store().message_id_by_remote(&account, "rep").unwrap().unwrap();
    // As if only the headers had synced: the content comes from the provider.
    engine.store().drop_blob(&id).unwrap();
    let slim = satchel_mime::Slim { raw: satchel_mime::strip_attachments(&raw).unwrap(), attachments: satchel_mime::parse(&raw).unwrap().attachments };
    fake.slim.lock().unwrap().insert("rep".into(), slim);
    let before = fake.raw_fetches.load(std::sync::atomic::Ordering::Relaxed);

    let detail: MessageDetail = serde_json::from_value(engine.handle(&Caller::User, Command::GetMessage { message_id: id.clone() }).await.unwrap()).unwrap();
    assert_eq!(detail.text_body.trim(), "The report is attached.");
    assert_eq!((detail.attachments[0].filename.as_str(), detail.attachments[0].size), ("report.pdf", 150_000));
    assert_eq!(fake.raw_fetches.load(std::sync::atomic::Ordering::Relaxed), before, "the full message never downloaded");
    let row = engine.store().message_row(&id).unwrap().unwrap();
    assert!(row.partial);
    assert_eq!(row.attachments[0].size, 150_000, "stored size comes from the server's structure");

    // Opening the attachment downloads the full message.
    let v = engine.handle(&Caller::User, Command::GetAttachment { message_id: id.clone(), attachment_id: row.attachments[0].id.clone() }).await.unwrap();
    assert_eq!(v["data_base64"].as_str().unwrap().len(), 200_000);
    assert_eq!(fake.raw_fetches.load(std::sync::atomic::Ordering::Relaxed), before + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_mail_and_removed_accounts_leave_no_stored_copies() {
    use satchel_core::blobs::{BlobStore, MemBlobStore};
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let blobs = Arc::new(MemBlobStore::default());
    let engine = Engine::with_parts(
        Arc::new(satchel_core::store::Store::open_in_memory().unwrap()),
        blobs.clone(),
        satchel_core::EngineOptions {
            data_dir: dir.path().to_path_buf(),
            mode: satchel_core::Mode::Full,
            secrets: Arc::new(satchel_core::secrets::MemSecretStore::default()),
            factory: Arc::new(FakeFactory(fake.clone())),
            start_workers: false,
        },
    )
    .unwrap();
    let acc: Account = serde_json::from_value(
        engine
            .handle(&Caller::User, Command::AddAccount { config: AccountConfig::Gmail { email: "me@x.test".into(), display_name: "Me".into() }, credential: Credential::Password { password: "x".into() } })
            .await
            .unwrap(),
    )
    .unwrap();
    wait_for(|| engine.store().message_id_by_remote(&acc.id, "r3").ok().flatten().is_some()).await;
    let count = || blobs.list(Duration::ZERO).unwrap().len();
    assert_eq!(count(), 3);

    let r1 = engine.store().message_id_by_remote(&acc.id, "r1").unwrap().unwrap();
    engine.store().delete_message(&r1).unwrap();
    assert_eq!(engine.free_unused_blobs().unwrap(), 1);
    assert_eq!(count(), 2);

    // Left behind by an earlier run.
    blobs.put(b"orphan").unwrap();
    assert_eq!(engine.sweep_orphan_blobs().unwrap(), 1);
    assert_eq!(count(), 2);

    engine.handle(&Caller::User, Command::RemoveAccount { account_id: acc.id.clone() }).await.unwrap();
    assert_eq!(count(), 0);
}
