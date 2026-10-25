//! Sending: at most once, Bcc where the provider needs it, and addresses that could inject headers.

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_send_goes_out_once_with_bcc_for_header_based_providers() {
    let (engine, fake, account, _dir) = setup().await;
    engine.handle(&Caller::User, Command::Send { draft: send_draft(&account) }).await.unwrap();
    wait_for(|| engine.store().list_outbox().unwrap().is_empty() && !fake.sent.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let sent = fake.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "sent exactly once");
    // The fake account is Gmail, which sends to the recipients in the headers.
    let text = String::from_utf8_lossy(&sent[0]);
    assert!(text.contains("Bcc: <secret@x.test>") || text.contains("Bcc: secret@x.test"), "Bcc header present: {text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unconfirmed_send_is_never_retried() {
    let (engine, fake, account, _dir) = setup().await;
    fake.sends_unconfirmed.store(true, std::sync::atomic::Ordering::Relaxed);
    engine.handle(&Caller::User, Command::Send { draft: send_draft(&account) }).await.unwrap();
    wait_for(|| engine.store().list_outbox().unwrap().iter().any(|o| o.status == "failed")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake.sent.lock().unwrap().len(), 1, "no automatic retry");
    let item = engine.store().list_outbox().unwrap().remove(0);
    assert!(item.last_error.unwrap().contains("check Sent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_cut_off_mid_flight_fails_instead_of_resending() {
    let (engine, fake, account, _dir) = setup().await;
    // As if Satchel quit while the send was in flight.
    let op = engine.store().enqueue_op(&account, "send", &json!({ "raw": "", "rcpts": ["bob@x.test"], "from": "me@x.test" }), "user").unwrap();
    engine.store().op_sending(&op).unwrap();
    engine.handle(&Caller::User, Command::SyncNow { account_id: Some(account.clone()) }).await.unwrap();
    wait_for(|| engine.store().list_outbox().unwrap().iter().any(|o| o.id == op && o.status == "failed")).await;
    assert!(fake.sent.lock().unwrap().is_empty());
}


#[tokio::test(flavor = "multi_thread")]
async fn addresses_that_could_inject_headers_are_refused() {
    let (engine, _fake, account, _dir) = setup().await;
    let a = agent(&engine, AgentPolicy::default()).await;
    let evil = Draft { account_id: account.clone(), to: vec![Address::new("bob@x.test")], cc: vec![Address::new("bob@x.test>\r\nBcc: evil@e.test\r\nX: <y")], subject: "Hi".into(), ..Default::default() };
    let err = engine.handle(&a, Command::CreateProposal { action: ProposalAction::Draft { draft: evil.clone() }, rationale: "r".into(), source_thread_ids: vec![] }).await.unwrap_err();
    assert!(err.message.contains("invalid address"), "{err:?}");
    assert!(engine.handle(&Caller::User, Command::Send { draft: evil }).await.is_err());
    // A half-typed address is fine in a saved draft, but not in a send.
    let typing = Draft { account_id: account.clone(), to: vec![Address::new("bo")], ..Default::default() };
    engine.handle(&Caller::User, Command::SaveDraft { draft: typing.clone() }).await.unwrap();
    assert!(engine.handle(&Caller::User, Command::Send { draft: typing }).await.is_err());
}
