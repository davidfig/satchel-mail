//! A deleted message stays deleted: it leaves its folder at once, and neither
//! a refresh nor a sync that read the server beforehand brings it back.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use satchel_api::MailboxRole::{Inbox, Junk, Sent, Trash};
use support::mail::*;

fn query(role: MailboxRole) -> Command {
    Command::ListThreads { query: ThreadQuery { role: Some(role), limit: Some(100), ..Default::default() } }
}

async fn subjects(engine: &Arc<Engine>, cmd: Command) -> Vec<String> {
    inbox_threads(engine.handle(&Caller::User, cmd).await.unwrap()).into_iter().map(|t| t.subject).collect()
}

async fn in_list(engine: &Arc<Engine>, role: MailboxRole) -> Vec<String> {
    subjects(engine, query(role)).await
}

async fn deliver_and_sync(engine: &Arc<Engine>, fake: &Fake, account: &str, mail: &[(&str, &str, &str)]) -> Vec<String> {
    for (rid, subject, mailbox) in mail {
        deliver(fake, rid, raw(rid, "Spammer <x@evil.test>", subject, None), mailbox);
    }
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    let last = mail.last().unwrap().0;
    wait_for(|| engine.store().message_id_by_remote(account, last).ok().flatten().is_some()).await;
    mail.iter().map(|(rid, ..)| engine.store().message_id_by_remote(account, rid).unwrap().unwrap()).collect()
}

/// Both kinds of account: Gmail's labels and IMAP's folders.
async fn each_account_kind() -> [(Arc<Engine>, Arc<Fake>, String, tempfile::TempDir); 2] {
    [setup_labels().await, setup().await]
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_from_spam_takes_it_out_of_spam() {
    for (engine, fake, account, _dir) in each_account_kind().await {
        let ids = deliver_and_sync(&engine, &fake, &account, &[("j1", "Win", "Junk"), ("j2", "Win again", "Junk")]).await;

        engine.handle(&Caller::User, Command::Trash { message_ids: vec![ids[0].clone()] }).await.unwrap();

        assert_eq!(in_list(&engine, Junk).await, vec!["Win again"], "the list re-read after a delete no longer has it");
        assert!(in_list(&engine, Trash).await.contains(&"Win".to_string()));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gmail_is_told_to_take_a_deleted_message_out_of_spam() {
    let (engine, fake, account, _dir) = setup_labels().await;
    let ids = deliver_and_sync(&engine, &fake, &account, &[("j1", "Win", "Junk")]).await;

    engine.handle(&Caller::User, Command::Trash { message_ids: ids }).await.unwrap();

    wait_for(|| !fake.labels.lock().unwrap().is_empty()).await;
    let (add, remove) = fake.labels.lock().unwrap()[0].clone();
    assert_eq!(add, vec!["Trash"]);
    assert!(remove.contains(&"Junk".to_string()), "still labelled Spam on the server: {remove:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn spam_and_delete_stays_gone_after_a_sync() {
    for (engine, fake, account, _dir) in each_account_kind().await {
        let ids = deliver_and_sync(&engine, &fake, &account, &[("j1", "Win", "Junk")]).await;

        // What the Spam button does on mail already in Spam.
        engine.handle(&Caller::User, Command::MarkSpam { message_ids: ids.clone() }).await.unwrap();
        engine.handle(&Caller::User, Command::Trash { message_ids: ids.clone() }).await.unwrap();
        assert!(in_list(&engine, Junk).await.is_empty());

        // Sync runs the sender rules, which mustn't file it under Spam again.
        wait_for(|| engine.store().list_outbox().unwrap().is_empty()).await;
        engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(in_list(&engine, Junk).await.is_empty());
        assert_eq!(in_list(&engine, Trash).await, vec!["Win"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gmail_keeps_a_deleted_messages_labels_but_only_trash_lists_it() {
    let (engine, fake, account, _dir) = setup_labels().await;
    fake.folders.lock().unwrap().push(("receipts".into(), "Receipts".into(), "Receipts".into()));
    let ids = deliver_and_sync(&engine, &fake, &account, &[("l1", "Your receipt", "receipts"), ("s1", "I wrote this", "Sent")]).await;
    let receipts = engine.store().mailbox_by_remote(&account, "receipts").unwrap().unwrap().id;
    let in_receipts = Command::ListThreads { query: ThreadQuery { mailbox_id: Some(receipts.clone()), limit: Some(100), ..Default::default() } };
    assert_eq!(subjects(&engine, in_receipts.clone()).await, vec!["Your receipt"]);

    engine.handle(&Caller::User, Command::Trash { message_ids: ids }).await.unwrap();

    assert!(subjects(&engine, in_receipts).await.is_empty());
    assert!(in_list(&engine, Sent).await.is_empty());
    assert_eq!(in_list(&engine, Trash).await.len(), 2);
    // The count sizes the list, so it has to agree.
    let count = engine.handle(&Caller::User, Command::CountThreads { query: ThreadQuery { mailbox_id: Some(receipts), ..Default::default() } }).await.unwrap();
    assert_eq!(count["total"], 0);
}

/// Runs `sync`, as a sync that read the server before the user acted would, on
/// another thread for as long as `act` takes.
async fn racing<S, A, F>(sync: S, act: A)
where
    S: Fn() + Send + 'static,
    A: FnOnce() -> F,
    F: std::future::Future<Output = ()>,
{
    let stop = Arc::new(AtomicBool::new(false));
    let running = stop.clone();
    let writer = std::thread::spawn(move || {
        while !running.load(Ordering::Relaxed) {
            sync();
        }
    });
    act().await;
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
}

/// `count` messages in Spam, with the op queue held so that what the user does
/// stays on its way to the server. Returns the messages' ids and the sync context.
async fn queued_up(count: usize) -> (Arc<Engine>, Arc<Fake>, String, tempfile::TempDir, Vec<String>, SyncCtx) {
    let (engine, fake, account, dir) = setup_labels().await;
    let rids: Vec<String> = (0..count).map(|i| format!("j{i}")).collect();
    let mail: Vec<(&str, &str, &str)> = rids.iter().map(|r| (r.as_str(), r.as_str(), "Junk")).collect();
    let ids = deliver_and_sync(&engine, &fake, &account, &mail).await;
    fake.hold_ops.store(true, Ordering::Relaxed);
    let ctx = fake.ctx.lock().unwrap().clone().unwrap();
    (engine, fake, account, dir, ids, ctx)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_during_a_delete_cant_put_messages_back() {
    let (engine, fake, _account, _dir, ids, ctx) = queued_up(40).await;
    let rids: Vec<String> = (0..ids.len()).map(|i| format!("j{i}")).collect();

    racing(
        move || {
            for r in &rids {
                ctx.set_memberships_remote(r, &[("Junk".into(), None)]).unwrap();
            }
        },
        || async { engine.handle(&Caller::User, Command::Trash { message_ids: ids.clone() }).await.unwrap(); },
    )
    .await;

    assert!(in_list(&engine, Junk).await.is_empty(), "a stale sync put deleted mail back in Spam");
    assert_eq!(in_list(&engine, Trash).await.len(), ids.len());
    fake.hold_ops.store(false, Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_during_a_read_cant_mark_messages_unread() {
    let (engine, fake, _account, _dir, ids, ctx) = queued_up(40).await;
    let rids: Vec<String> = (0..ids.len()).map(|i| format!("j{i}")).collect();

    racing(
        move || {
            for r in &rids {
                ctx.set_flags_remote(r, Flags::default()).unwrap();
            }
        },
        || async { engine.handle(&Caller::User, Command::SetFlags { message_ids: ids.clone(), seen: Some(true), flagged: None }).await.unwrap(); },
    )
    .await;

    for id in &ids {
        assert!(engine.store().message_ref(id).unwrap().unwrap().flags.seen, "a stale sync undid marking it read");
    }
    fake.hold_ops.store(false, Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_during_a_permanent_delete_cant_store_messages_again() {
    let (engine, fake, account, _dir) = setup_labels().await;
    let rids: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
    let mail: Vec<(&str, &str, &str)> = rids.iter().map(|r| (r.as_str(), r.as_str(), "Trash")).collect();
    let ids = deliver_and_sync(&engine, &fake, &account, &mail).await;
    fake.hold_ops.store(true, Ordering::Relaxed);
    let ctx = fake.ctx.lock().unwrap().clone().unwrap();

    let stale = rids.clone();
    racing(
        move || {
            for r in &stale {
                ctx.put_raw_message(r, None, &raw(r, "Spammer <x@evil.test>", r, None), Flags::default(), &[("Trash".into(), None)], None).unwrap();
            }
        },
        || async { engine.handle(&Caller::User, Command::Trash { message_ids: ids.clone() }).await.unwrap(); },
    )
    .await;

    for r in &rids {
        assert!(engine.store().message_id_by_remote(&account, r).unwrap().is_none(), "{r} was stored again while being deleted");
    }
    fake.hold_ops.store(false, Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_catching_up_leaves_a_deleted_message_deleted() {
    let (engine, fake, account, _dir, ids, ctx) = queued_up(1).await;

    engine.handle(&Caller::User, Command::Trash { message_ids: ids.clone() }).await.unwrap();
    // Still on its way: the server says Spam, and the delete stands.
    ctx.set_memberships_remote("j0", &[("Junk".into(), None)]).unwrap();
    assert!(in_list(&engine, Junk).await.is_empty());

    // It arrives, and the server now says what the user did.
    fake.hold_ops.store(false, Ordering::Relaxed);
    wait_for(|| engine.store().list_outbox().unwrap().is_empty()).await;
    ctx.set_memberships_remote("j0", &[("Trash".into(), None)]).unwrap();
    assert!(in_list(&engine, Junk).await.is_empty());
    assert!(in_role(&engine, &account, "j0", Trash));
    assert!(!in_role(&engine, &account, "j0", Inbox));
}
