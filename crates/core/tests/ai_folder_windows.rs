//! The AI folder over time: mail and attachments leaving it as they age past
//! its date limits (0078), and passes that write only what changed (0079).

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn threads_leave_the_folder_once_they_age_past_the_date_limit() {
    let (engine, fake, account, dir) = setup().await;
    let mut s = engine.settings();
    s.mirror.since_days = Some(1);
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    // Inside the limit by three seconds.
    let date = (chrono::Utc::now() - chrono::Duration::days(1) + chrono::Duration::seconds(3)).to_rfc2822();
    let raw = format!("From: Dana <dana@x.test>\r\nTo: me@x.test\r\nSubject: Fading\r\nMessage-ID: <fade@x.test>\r\nDate: {date}\r\n\r\nSoon gone.\r\n");
    deliver(&fake, "fade", raw.into_bytes(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "fade").ok().flatten().is_some()).await;
    engine.mirror_pass(true).unwrap();
    let id = engine.store().message_id_by_remote(&account, "fade").unwrap().unwrap();
    let thread = engine.store().thread_of(&id).unwrap().unwrap();
    let folder = dir.path().join("mirror").join(engine.store().mirror_path(&thread).unwrap().unwrap());
    assert!(folder.join("thread.md").is_file());

    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    // Nothing about the thread changed, so a pass alone leaves it.
    engine.mirror_pass(false).unwrap();
    assert!(folder.exists());
    // The daily sweep finds it.
    engine.recheck_mirror();
    engine.mirror_pass(false).unwrap();
    assert!(!folder.exists());
    assert_eq!(engine.store().mirror_path(&thread).unwrap(), None);
}

/// A message from `age` ago with a PDF (as base64) attached.
fn with_pdf(id: &str, age: chrono::Duration, name: &str, pdf: &str) -> Vec<u8> {
    let date = (chrono::Utc::now() - age).to_rfc2822();
    format!(
        "From: Bob <bob@x.test>\r\nTo: me@x.test\r\nSubject: Report {id}\r\nMessage-ID: <{id}@x.test>\r\nDate: {date}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nAttached.\r\n--b\r\nContent-Type: application/pdf; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b--\r\n"
    )
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
async fn recent_attachments_are_downloaded_for_the_folder_alone() {
    let (engine, fake, account, dir) = setup().await;
    let mut s = engine.settings();
    s.mirror.include_attachments = true;
    assert_eq!(s.mirror.attachment_days, Some(90), "the default");
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s.clone() }).await.unwrap();
    let big = "QUFB".repeat(50_000);
    let days = chrono::Duration::days;
    // "small" is inside the window by a few seconds.
    let mail = [
        ("new", with_pdf("new", days(1), "report.pdf", &big)),
        ("old", with_pdf("old", days(200), "old.pdf", &big)),
        ("small", with_pdf("small", days(90) - chrono::Duration::seconds(4), "note.pdf", "QUFB")),
    ];
    for (id, raw) in &mail {
        deliver(&fake, id, raw.clone(), "INBOX");
    }
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| mail.iter().all(|(id, _)| engine.store().message_row(&engine.store().message_id_by_remote(&account, id).ok().flatten().unwrap_or_default()).ok().flatten().is_some_and(|r| r.blob_hash.is_some()))).await;
    let id = |rid: &str| engine.store().message_id_by_remote(&account, rid).unwrap().unwrap();
    // Opened in the app, so its stored copy is whole.
    engine.handle(&Caller::User, Command::GetAttachment { message_id: id("small"), attachment_id: "0".into() }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    let root = dir.path().join("mirror");
    let folder = |rid: &str| root.join(engine.store().mirror_path(&engine.store().thread_of(&id(rid)).unwrap().unwrap()).unwrap().unwrap()).join("attachments");
    assert!(engine.store().message_row(&id("new")).unwrap().unwrap().partial, "stored without the PDF");
    // A whole stored copy's attachments are written with the thread.
    assert_eq!(std::fs::read(folder("small").join("1-note.pdf")).unwrap(), b"AAA");
    assert!(!folder("new").exists(), "not downloaded yet");

    let mut failed = Default::default();
    assert_eq!(engine.fetch_ai_attachments(&mut failed).await, 1, "only the recent slim one wants downloading");
    assert_eq!(std::fs::read(folder("new").join("1-report.pdf")).unwrap().len(), 150_000);
    assert!(engine.store().message_row(&id("new")).unwrap().unwrap().partial, "the stored copy stays slim");
    assert!(!folder("old").exists(), "outside the attachment window, inside the text one");
    assert_eq!(engine.fetch_ai_attachments(&mut failed).await, 0);
    assert!(failed.is_empty());

    // Rewriting the thread keeps what was downloaded.
    engine.store().reset_mirrored().unwrap();
    engine.mirror_pass(true).unwrap();
    assert!(folder("new").join("1-report.pdf").is_file());
    assert_eq!(engine.fetch_ai_attachments(&mut failed).await, 0, "nothing to download again");

    // Once the window has passed one, it goes: the daily sweep finds it
    // without any change to its thread.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    engine.mirror_pass(true).unwrap();
    assert!(folder("small").join("1-note.pdf").is_file(), "a pass alone leaves it");
    engine.recheck_mirror();
    engine.mirror_pass(true).unwrap();
    assert!(!folder("small").join("1-note.pdf").exists());
    assert!(folder("small").parent().unwrap().join("thread.md").is_file(), "the thread stays");
    assert!(folder("new").join("1-report.pdf").is_file());

    // Turning attachments off removes the rest.
    s.mirror.include_attachments = false;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    assert!(!folder("new").join("1-report.pdf").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_pass_writes_only_what_changed() {
    let (engine, fake, account, dir) = setup().await;
    engine.mirror_pass(true).unwrap();
    let base = dir.path().join("mirror");
    let (index, agenda) = (base.join("mail/me@x.test/INDEX.md"), base.join("calendar/AGENDA.md"));
    assert!(index.is_file() && agenda.is_file());
    // Nothing they're built from changed, so nothing is written.
    std::fs::remove_file(&index).unwrap();
    std::fs::remove_file(&agenda).unwrap();
    engine.mirror_pass(true).unwrap();
    assert!(!index.exists() && !agenda.exists());

    // New mail brings the mail indexes back; the agenda waits for its own
    // change, or the next full write.
    deliver(&fake, "late", raw("late", "Dana <dana@x.test>", "Late arrival", None), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "late").ok().flatten().is_some()).await;
    engine.mirror_pass(true).unwrap();
    assert!(std::fs::read_to_string(&index).unwrap().contains("Late arrival"));
    assert!(!agenda.exists());
    engine.refresh_mirror();
    engine.mirror_pass(true).unwrap();
    assert!(agenda.is_file());
}
