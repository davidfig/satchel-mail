//! Send-as addresses (0072): which From a message gets, which address a
//! reply defaults to, and which SMTP server a send goes through.

mod support;

use support::mail::*;

fn alias(email: &str, smtp: Option<SmtpServer>) -> SendAs {
    SendAs { email: email.into(), display_name: "Satchel".into(), smtp }
}

fn cloudflare() -> SmtpServer {
    SmtpServer { host: "smtp.relay.test".into(), port: 465, security: Security::Tls, username: "api_token".into() }
}

async fn set(engine: &Arc<Engine>, account: &str, list: Vec<SendAs>, creds: &[(&str, &str)]) -> Result<Account, ApiError> {
    let smtp_credentials = creds.iter().map(|(e, p)| (e.to_string(), Credential::Password { password: p.to_string() })).collect();
    engine
        .handle(&Caller::User, Command::SetSendAs { account_id: account.into(), send_as: list, smtp_credentials })
        .await
        .map(|v| serde_json::from_value(v).unwrap())
}

async fn send_and_wait(engine: &Arc<Engine>, fake: &Fake, draft: Draft) -> (String, (String, bool, Option<SmtpRelay>)) {
    let before = fake.sent.lock().unwrap().len();
    engine.handle(&Caller::User, Command::Send { draft }).await.unwrap();
    wait_for(|| fake.sent.lock().unwrap().len() > before).await;
    let raw = String::from_utf8_lossy(&fake.sent.lock().unwrap()[before]).into_owned();
    (raw, fake.routes.lock().unwrap()[before].clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_goes_out_from_the_chosen_send_as_address() {
    let (engine, fake, account, _dir) = setup().await;
    set(&engine, &account, vec![alias("hello@satchel.test", None)], &[]).await.unwrap();
    let mut d = send_draft(&account);
    d.from = Some(Address::new("Hello@Satchel.test"));
    let (raw, (from, send_as, smtp)) = send_and_wait(&engine, &fake, d).await;
    assert!(raw.contains("From: \"Satchel\" <hello@satchel.test>"), "{raw}");
    assert!(raw.contains("@satchel.test>\r\n") && raw.contains("Message-ID: <"), "Message-ID in the alias's domain: {raw}");
    assert_eq!((from.as_str(), send_as, smtp.is_none()), ("hello@satchel.test", true, true));

    // No From: the account's own address.
    let (raw, (from, send_as, _)) = send_and_wait(&engine, &fake, send_draft(&account)).await;
    assert!(raw.contains("<me@x.test>") && raw.contains("From: \"Me\""), "{raw}");
    assert_eq!((from.as_str(), send_as), ("me@x.test", false));

    // An address that isn't the account's is refused, from the user or an agent.
    let mut d = send_draft(&account);
    d.from = Some(Address::new("ceo@bank.test"));
    assert!(engine.handle(&Caller::User, Command::Send { draft: d.clone() }).await.is_err());
    let a = agent(&engine, AgentPolicy::default()).await;
    let err = engine.handle(&a, Command::CreateProposal { action: ProposalAction::Draft { draft: d }, rationale: "r".into(), source_thread_ids: vec![] }).await.unwrap_err();
    assert!(err.message.contains("isn't one of"), "{err:?}");
    // Agents can't change send-as addresses.
    assert!(engine.handle(&a, Command::SetSendAs { account_id: account.clone(), send_as: vec![], smtp_credentials: Default::default() }).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn replies_default_to_the_address_the_mail_was_sent_to() {
    let (engine, fake, account, _dir) = setup().await;
    set(&engine, &account, vec![alias("hello@satchel.test", None), alias("david@satchel.test", None)], &[]).await.unwrap();
    let forwarded = b"From: Zed <zed@x.test>\r\nTo: hello@satchel.test, Me <me@x.test>\r\nCc: david@satchel.test\r\nSubject: Plan\r\nMessage-ID: <f1@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\r\nHi\r\n".to_vec();
    let bcc = b"Delivered-To: me@x.test\r\nDelivered-To: david@satchel.test\r\nFrom: List <list@x.test>\r\nTo: everyone@x.test\r\nSubject: News\r\nMessage-ID: <f2@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\r\nHi\r\n".to_vec();
    deliver(&fake, "f1", forwarded, "INBOX");
    deliver(&fake, "f2", bcc, "INBOX");
    deliver(&fake, "f3", raw("f3", "Bob <bob@x.test>", "Direct", None), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "f3").unwrap().is_some()).await;
    let id = |rid| engine.store().message_id_by_remote(&account, rid).unwrap().unwrap();
    let reply = |rid, mode| {
        let engine = engine.clone();
        let mid = id(rid);
        async move { serde_json::from_value::<Draft>(engine.handle(&Caller::User, Command::ComposeReply { message_id: mid, mode }).await.unwrap()).unwrap() }
    };

    let r = reply("f1", ReplyMode::ReplyAll).await;
    assert_eq!(r.from.as_ref().map(|a| a.email.as_str()), Some("hello@satchel.test"));
    // Reply all leaves out every one of my addresses.
    let all: Vec<&str> = r.to.iter().chain(&r.cc).map(|a| a.email.as_str()).collect();
    assert_eq!(all, vec!["zed@x.test"], "{all:?}");
    // Only the delivery headers name it.
    let body_ready = || engine.store().message_row(&id("f2")).unwrap().unwrap().blob_hash.is_some();
    wait_for(body_ready).await;
    assert_eq!(reply("f2", ReplyMode::Reply).await.from.map(|a| a.email), Some("david@satchel.test".into()));
    // Sent to the account's own address: no send-as From.
    assert!(reply("f3", ReplyMode::Reply).await.from.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_as_address_with_its_own_server_sends_through_it() {
    let (engine, fake, account, _dir) = setup_imap().await;
    // A new server needs its password, and is checked before it's saved.
    let list = vec![alias("hello@satchel.test", Some(cloudflare())), alias("plain@satchel.test", None)];
    let e = set(&engine, &account, list.clone(), &[]).await.unwrap_err();
    assert!(e.message.contains("password"), "{e:?}");
    let e = set(&engine, &account, list.clone(), &[("hello@satchel.test", "wrong")]).await.unwrap_err();
    assert!(e.message.contains("535"), "{e:?}");
    let acc = set(&engine, &account, list.clone(), &[("HELLO@satchel.test", "token-1")]).await.unwrap();
    assert_eq!(acc.send_as.len(), 2);
    assert_eq!(fake.smtp_tests.lock().unwrap().len(), 2);
    // Saving again unchanged doesn't check (or need) the password again.
    set(&engine, &account, list.clone(), &[]).await.unwrap();
    assert_eq!(fake.smtp_tests.lock().unwrap().len(), 2);

    let mut d = send_draft(&account);
    d.from = Some(Address::new("hello@satchel.test"));
    let (raw, (from, send_as, smtp)) = send_and_wait(&engine, &fake, d).await;
    assert!(raw.contains("From: \"Satchel\" <hello@satchel.test>"));
    // SMTP delivers to the envelope, so no Bcc header.
    assert!(!raw.contains("Bcc:"), "{raw}");
    let smtp = smtp.expect("routed through the alias's server");
    assert_eq!((from.as_str(), send_as, smtp.server.host.as_str(), smtp.password.as_str()), ("hello@satchel.test", true, "smtp.relay.test", "token-1"));
    assert!(!format!("{smtp:?}").contains("token-1"), "Debug never prints the password");

    // Without its own server: the account's SMTP.
    let mut d = send_draft(&account);
    d.from = Some(Address::new("plain@satchel.test"));
    let (_, (from, send_as, smtp)) = send_and_wait(&engine, &fake, d).await;
    assert_eq!((from.as_str(), send_as, smtp.is_none()), ("plain@satchel.test", true, true));

    // Removing the server deletes its password; adding it back needs it again.
    set(&engine, &account, vec![alias("hello@satchel.test", None)], &[]).await.unwrap();
    assert!(set(&engine, &account, vec![alias("hello@satchel.test", Some(cloudflare()))], &[]).await.is_err());
}
