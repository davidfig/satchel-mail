//! Sender rules, phishing warnings and spam invitations.

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn sender_rules_outrank_the_server_filter() {
    let (engine, fake, account, _dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();

    // Agents can't decide what's spam.
    let a = agent(&engine, AgentPolicy::default()).await;
    let err = engine.handle(&a, Command::MarkSpam { message_ids: vec![invoice.clone()] }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);

    let r = engine.handle(&Caller::User, Command::MarkSpam { message_ids: vec![invoice.clone()] }).await.unwrap();
    assert_eq!(r["senders"], json!(["bill@evil.test"]));
    assert!(in_role(&engine, &account, "r3", MailboxRole::Junk));

    // New mail from the sender lands in the inbox; the rule moves it.
    deliver(&fake, "r4", raw("m4", "Vendor <Bill@Evil.test>", "Invoice again", None), "INBOX");
    // The server puts mail from a real person in Junk.
    deliver(&fake, "r5", raw("m5", "Carol <carol@x.test>", "Lunch?", None), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "r4", MailboxRole::Junk)).await;
    assert!(in_role(&engine, &account, "r5", MailboxRole::Junk), "no rule for Carol yet");

    // Not spam moves her back, and her later mail never stays in Junk.
    let lunch = engine.store().message_id_by_remote(&account, "r5").unwrap().unwrap();
    engine.handle(&Caller::User, Command::MarkNotSpam { message_ids: vec![lunch] }).await.unwrap();
    assert!(in_role(&engine, &account, "r5", MailboxRole::Inbox));
    deliver(&fake, "r6", verified(raw("m6", "Carol <carol@x.test>", "Lunch, take two", None)), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "r6", MailboxRole::Inbox)).await;

    let rules: Vec<SenderRule> = serde_json::from_value(engine.handle(&Caller::User, Command::ListSenderRules).await.unwrap()).unwrap();
    assert_eq!(rules.len(), 2);
    // Removing a rule lets the server's filter decide again.
    engine.handle(&Caller::User, Command::SetSenderRule { email: "bill@evil.test".into(), verdict: None }).await.unwrap();
    deliver(&fake, "r7", raw("m7", "Vendor <bill@evil.test>", "Third time", None), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "r7").ok().flatten().is_some()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(in_role(&engine, &account, "r7", MailboxRole::Inbox));
}

#[tokio::test(flavor = "multi_thread")]
async fn emptying_spam_deletes_only_spam() {
    let (engine, fake, account, _dir) = setup().await;
    deliver(&fake, "j1", raw("j1", "Spammer <x@evil.test>", "Win", None), "Junk");
    deliver(&fake, "j2", raw("j2", "Spammer <x@evil.test>", "Win again", None), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "j2").ok().flatten().is_some()).await;

    // Agents can't, and only Spam and Trash can be emptied.
    let a = agent(&engine, AgentPolicy::default()).await;
    let err = engine.handle(&a, Command::EmptyMailbox { mailbox_id: None, role: Some(MailboxRole::Junk) }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let err = engine.handle(&Caller::User, Command::EmptyMailbox { mailbox_id: None, role: Some(MailboxRole::Inbox) }).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    let r = engine.handle(&Caller::User, Command::EmptyMailbox { mailbox_id: None, role: Some(MailboxRole::Junk) }).await.unwrap();
    assert_eq!(r["deleted"], json!(2));
    assert!(engine.store().message_id_by_remote(&account, "j1").unwrap().is_none());
    assert!(in_role(&engine, &account, "r1", MailboxRole::Inbox), "the inbox is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn warns_about_impersonation_and_deceptive_links() {
    let (engine, fake, account, _dir) = setup().await;
    // The user has written to Bob, so "Bob" from another address is suspicious.
    deliver(&fake, "s1", raw("s1", "Me <me@x.test>", "Re: Kickoff", Some("m1")).replace_to("Bob <bob@x.test>"), "Sent");
    let mut spoof = raw("s2", "Bob <bob.office@freemail.test>", "Quick favor", None);
    spoof = String::from_utf8(spoof).unwrap().replacen("\r\n\r\n", "\r\nAuthentication-Results: mx.google.com; spf=fail; dkim=none; dmarc=fail\r\n\r\n", 1).into_bytes();
    deliver(&fake, "s2", spoof, "INBOX");
    let html = "From: Bank <alerts@bank-secure.test>\r\nTo: me@x.test\r\nSubject: Verify\r\nMessage-ID: <s3@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\
        List-Unsubscribe: <https://bank-secure.test/u>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\nContent-Type: text/html\r\n\r\n\
        <p><a href=\"https://bank-secure.test/login\">https://www.bank.com/login</a></p>\r\n";
    deliver(&fake, "s3", html.as_bytes().to_vec(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "s3").ok().flatten().is_some()).await;

    let get = |rid: &str| {
        let id = engine.store().message_id_by_remote(&account, rid).unwrap().unwrap();
        let engine = engine.clone();
        async move { serde_json::from_value::<MessageDetail>(engine.handle(&Caller::User, Command::GetMessage { message_id: id }).await.unwrap()).unwrap() }
    };
    let m = get("s2").await;
    let kinds: Vec<WarningKind> = m.warnings.iter().map(|w| w.kind).collect();
    assert_eq!(kinds, vec![WarningKind::Impersonation, WarningKind::AuthFailed], "{:?}", m.warnings);
    assert!(m.warnings[0].message.contains("bob@x.test"));

    let m = get("s3").await;
    assert_eq!(m.warnings.len(), 1);
    assert_eq!(m.warnings[0].kind, WarningKind::DeceptiveLink);
    assert!(m.unsubscribe.as_ref().is_some_and(|u| u.one_click));

    // Bob himself gets no warnings.
    assert!(get("r1").await.warnings.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn marking_spam_silently_removes_its_invitation() {
    let (engine, fake, account, _dir) = setup().await;
    // The server auto-added the unanswered invite; the user accepted the other one.
    let unanswered = invite_ics("spam-1", "NEEDS-ACTION");
    let accepted = invite_ics("spam-2", "ACCEPTED");
    deliver(&fake, "i1", invite_mail("i1", &unanswered), "INBOX");
    deliver(&fake, "i2", invite_mail("i2", &accepted), "INBOX");
    fake.events.lock().unwrap().extend([("e1".to_string(), unanswered), ("e2".to_string(), accepted)]);
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().events_by_uid(&account, "spam-2").map(|v| !v.is_empty()).unwrap_or(false)
        && engine.store().message_id_by_remote(&account, "i2").ok().flatten().is_some()).await;

    let ids: Vec<String> = ["i1", "i2"].iter().map(|r| engine.store().message_id_by_remote(&account, r).unwrap().unwrap()).collect();
    let r = engine.handle(&Caller::User, Command::MarkSpam { message_ids: ids }).await.unwrap();
    assert_eq!(r["invites_removed"], json!(1));
    assert!(engine.store().events_by_uid(&account, "spam-1").unwrap().is_empty());
    assert_eq!(engine.store().events_by_uid(&account, "spam-2").unwrap().len(), 1, "answered invitations stay");
    wait_for(|| !fake.cal_deletes.lock().unwrap().is_empty()).await;
    assert_eq!(*fake.cal_deletes.lock().unwrap(), vec![false], "the organizer is never notified");

    // An invite the server adds later, from the now-blocked sender, comes off after the next sync.
    let later = invite_ics("spam-3", "NEEDS-ACTION");
    deliver(&fake, "i3", invite_mail("i3", &later), "INBOX");
    fake.events.lock().unwrap().push(("e3".into(), later));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| fake.cal_deletes.lock().unwrap().len() == 2).await;
    assert!(engine.store().events_by_uid(&account, "spam-3").unwrap().is_empty());
    assert!(in_role(&engine, &account, "i3", MailboxRole::Junk));
}


#[tokio::test(flavor = "multi_thread")]
async fn not_spam_rules_leave_spoofed_mail_in_junk() {
    let (engine, fake, account, _dir) = setup().await;
    engine.handle(&Caller::User, Command::SetSenderRule { email: "carol@x.test".into(), verdict: Some(SenderVerdict::NotSpam) }).await.unwrap();
    // Someone else using Carol's address: her domain's DMARC check failed.
    let mut spoof = b"Authentication-Results: mx.google.com; spf=fail smtp.mailfrom=x.test; dmarc=fail header.from=x.test\r\n".to_vec();
    spoof.extend(raw("m8", "Carol <carol@x.test>", "Urgent wire transfer", None));
    deliver(&fake, "r8", spoof, "Junk");
    // No results at all: nothing shows the mail is really from her.
    deliver(&fake, "r10", raw("m10", "Carol <carol@x.test>", "Gift cards", None), "Junk");
    // DKIM signed by another domain isn't aligned with her address.
    let mut other = b"Authentication-Results: mx.google.com; spf=pass smtp.mailfrom=bulk.test; dkim=pass header.d=bulk.test\r\n".to_vec();
    other.extend(raw("m11", "Carol <carol@x.test>", "Invoice", None));
    deliver(&fake, "r11", other, "Junk");
    deliver(&fake, "r9", verified(raw("m9", "Carol <carol@x.test>", "Lunch?", None)), "Junk");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "r9", MailboxRole::Inbox)).await;
    for rid in ["r8", "r10", "r11"] {
        assert!(in_role(&engine, &account, rid, MailboxRole::Junk), "{rid}: unverified mail stays in Junk");
    }
}

/// A message as the receiving server verified it: DKIM and DMARC pass for x.test.
fn verified(raw: Vec<u8>) -> Vec<u8> {
    let mut out = b"Authentication-Results: mx.google.com; spf=pass smtp.mailfrom=x.test; dkim=pass header.d=x.test; dmarc=pass header.from=x.test\r\n".to_vec();
    out.extend(raw);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_a_shared_sender_keeps_invitations_others_organize() {
    let (engine, fake, account, _dir) = setup().await;
    engine.handle(&Caller::User, Command::SetSenderRule { email: "notify@calendar.test".into(), verdict: Some(SenderVerdict::Spam) }).await.unwrap();
    // A real invitation from Bob, relayed by a notification service the user blocked.
    let ics = invite_ics("real-1", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:bob@x.test");
    let mail = String::from_utf8(invite_mail("n1", &ics)).unwrap().replace("From: Prize <noreply@prize.test>", "From: Calendar <notify@calendar.test>");
    deliver(&fake, "n1", mail.into_bytes(), "INBOX");
    fake.events.lock().unwrap().push(("n1e".into(), ics));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| in_role(&engine, &account, "n1", MailboxRole::Junk) && !engine.store().events_by_uid(&account, "real-1").unwrap().is_empty()).await;
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(engine.store().events_by_uid(&account, "real-1").unwrap().len(), 1, "Bob's meeting stays on the calendar");
    assert!(fake.cal_deletes.lock().unwrap().is_empty());
}
