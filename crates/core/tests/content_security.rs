//! Untrusted mail content that tries to make Satchel act: unsubscribe links
//! that add recipients, invitations reusing a real meeting's UID, and image
//! trust claimed through a forged From.

mod support;

use support::mail::*;

async fn get(engine: &Arc<Engine>, account: &str, rid: &str) -> MessageDetail {
    let id = engine.store().message_id_by_remote(account, rid).unwrap().unwrap();
    serde_json::from_value(engine.handle(&Caller::User, Command::GetMessage { message_id: id }).await.unwrap()).unwrap()
}

async fn sync_until(engine: &Arc<Engine>, account: &str, rid: &str) {
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(account, rid).ok().flatten().is_some()).await;
}

fn with_header(raw: Vec<u8>, header: &str) -> Vec<u8> {
    let mut out = format!("{header}\r\n").into_bytes();
    out.extend(raw);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_mailto_shows_and_sends_one_address() {
    let (engine, fake, account, _dir) = setup().await;
    // Percent-encoded "a@x>, <boss@corp": refused, so no email option at all.
    deliver(&fake, "u1", with_header(raw("u1", "News <news@shop.test>", "Deals", None), "List-Unsubscribe: <mailto:u@shop.test%3E,%20%3Cboss@corp.test?subject=I%20quit>"), "INBOX");
    deliver(&fake, "u2", with_header(raw("u2", "News <news@shop.test>", "More deals", None), "List-Unsubscribe: <mailto:leave@lists.mailer.test?subject=Stop%0D%0ABcc:%20boss@corp.test>"), "INBOX");
    sync_until(&engine, &account, "u2").await;
    assert!(get(&engine, &account, "u1").await.unsubscribe.is_none());

    let m = get(&engine, &account, "u2").await;
    let e = m.unsubscribe.unwrap().email.unwrap();
    assert_eq!((e.to.as_str(), e.subject.as_str(), e.body.as_str()), ("leave@lists.mailer.test", "Stop Bcc: boss@corp.test", "unsubscribe"));
    assert!(e.other_domain, "the list is on another domain than the sender");
    let r = engine.handle(&Caller::User, Command::Unsubscribe { message_id: m.summary.id.clone() }).await.unwrap();
    assert_eq!(r["to"], json!("leave@lists.mailer.test"));
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| !fake.sent.lock().unwrap().is_empty()).await;
    let sent = String::from_utf8_lossy(&fake.sent.lock().unwrap()[0]).into_owned();
    assert!(!sent.to_ascii_lowercase().contains("\r\nbcc:"), "{sent}");
    let p = satchel_mime::parse(sent.as_bytes()).unwrap();
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), vec!["leave@lists.mailer.test"]);
    assert!(p.cc.is_empty() && p.bcc.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invitation_from_another_organizer_cant_answer_a_real_meeting() {
    let (engine, fake, account, _dir) = setup().await;
    // Bob's real meeting is on the synced calendar.
    let real = invite_ics("meet-1", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:bob@x.test").replace("Claim your prize", "Board meeting");
    fake.events.lock().unwrap().push(("e1".into(), real));
    // Someone else sends an invitation with the same UID.
    let fake_ics = invite_ics("meet-1", "NEEDS-ACTION").replace("Claim your prize", "Optional social, ignore");
    deliver(&fake, "f1", invite_mail("f1", &fake_ics), "INBOX");
    sync_until(&engine, &account, "f1").await;
    wait_for(|| !engine.store().events_by_uid(&account, "meet-1").unwrap().is_empty()).await;

    let m = get(&engine, &account, "f1").await;
    let inv = m.invite.unwrap();
    assert_eq!(inv.event_id, None, "shown as a new invitation, not as Bob's meeting");
    engine.handle(&Caller::User, Command::RespondToEmailInvite { message_id: m.summary.id.clone(), response: PartStat::Declined }).await.unwrap();
    let stored = engine.store().events_by_uid(&account, "meet-1").unwrap();
    assert_eq!(stored.len(), 1, "no copy shares the real meeting's UID");
    assert!(stored[0].ics.contains("PARTSTAT=NEEDS-ACTION") && stored[0].ics.contains("Board meeting"), "the real meeting is untouched");

    // An invitation whose ORGANIZER isn't its sender: no reply goes to that organizer.
    let other = invite_ics("meet-2", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:ceo@corp.test");
    deliver(&fake, "f2", invite_mail("f2", &other), "INBOX");
    sync_until(&engine, &account, "f2").await;
    let m = get(&engine, &account, "f2").await;
    let err = engine.handle(&Caller::User, Command::RespondToEmailInvite { message_id: m.summary.id, response: PartStat::Accepted }).await.unwrap_err();
    assert!(err.message.contains("ceo@corp.test"), "{}", err.message);
    assert!(engine.store().events_by_uid(&account, "meet-2").unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn accepting_a_new_invitation_stores_only_its_series() {
    let (engine, fake, account, _dir) = setup().await;
    let ics = invite_ics("new-1", "NEEDS-ACTION")
        .replace("END:VEVENT", "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:smuggled\r\nDTSTART:20261102T150000Z\r\nSUMMARY:Extra\r\nEND:VEVENT");
    deliver(&fake, "n1", invite_mail("n1", &ics), "INBOX");
    sync_until(&engine, &account, "n1").await;
    let m = get(&engine, &account, "n1").await;
    engine.handle(&Caller::User, Command::RespondToEmailInvite { message_id: m.summary.id, response: PartStat::Accepted }).await.unwrap();
    let stored = engine.store().events_by_uid(&account, "new-1").unwrap();
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].ics.contains("VALARM") && !stored[0].ics.contains("smuggled") && !stored[0].ics.contains("METHOD"), "{}", stored[0].ics);
    assert!(stored[0].ics.contains("PARTSTAT=ACCEPTED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn image_trust_needs_an_authentic_sender() {
    let (engine, fake, account, _dir) = setup().await;
    let mut s = engine.settings();
    s.remote_image_senders = vec!["carol@x.test".into()];
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let html = |id: &str| {
        format!(
            "From: Carol <carol@x.test>\r\nTo: me@x.test\r\nSubject: Pics {id}\r\nMessage-ID: <{id}@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\
             Content-Type: text/html\r\n\r\n<p>Hi <img src=\"https://track.evil.test/{id}.png\"></p>\r\n"
        )
        .into_bytes()
    };
    deliver(&fake, "p1", with_header(html("p1"), "Authentication-Results: mx.google.com; spf=pass smtp.mailfrom=x.test; dkim=pass header.d=x.test; dmarc=pass header.from=x.test"), "INBOX");
    deliver(&fake, "p2", with_header(html("p2"), "Authentication-Results: mx.google.com; spf=fail smtp.mailfrom=x.test; dkim=none; dmarc=fail header.from=x.test"), "INBOX");
    // No results at all, and a DMARC failure hidden behind a "pass" in a comment.
    deliver(&fake, "p3", html("p3"), "INBOX");
    deliver(&fake, "p4", with_header(html("p4"), "Authentication-Results: mx.google.com; spf=softfail (domain of dkim=pass.dmarc=pass@x.test) smtp.mailfrom=dkim=pass.dmarc=pass@x.test; dmarc=fail header.from=x.test"), "INBOX");
    // The right results from someone else's server: a forged header.
    deliver(&fake, "p5", with_header(html("p5"), "Authentication-Results: mx.evil.test; spf=pass smtp.mailfrom=x.test; dkim=pass header.d=x.test; dmarc=pass header.from=x.test"), "INBOX");
    // A forged pass below the header the real server added on top.
    let below = with_header(html("p6"), "Authentication-Results: mx.google.com; dkim=pass header.d=x.test; dmarc=pass header.from=x.test");
    deliver(&fake, "p6", with_header(below, "Authentication-Results: mx.google.com; spf=softfail smtp.mailfrom=x.test; dkim=none; dmarc=none header.from=x.test"), "INBOX");
    sync_until(&engine, &account, "p6").await;
    let real = get(&engine, &account, "p1").await;
    assert!(real.remote_images_allowed && !real.remote_images_blocked, "Gmail's own mx.google.com header is trusted");
    for id in ["p2", "p3", "p4", "p5", "p6"] {
        let forged = get(&engine, &account, id).await;
        assert!(!forged.remote_images_allowed && forged.remote_images_blocked, "{id}: an unverified From doesn't get the sender's image trust");
        assert!(!forged.html_body.unwrap_or_default().contains(&format!("track.evil.test/{id}.png")));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn always_load_images_skips_spam() {
    let (engine, fake, account, _dir) = setup().await;
    let mut s = engine.settings();
    s.load_remote_images = true;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    engine.handle(&Caller::User, Command::SetSenderRule { email: "spammer@y.test".into(), verdict: Some(SenderVerdict::Spam) }).await.unwrap();
    let html = |id: &str, from: &str| {
        format!(
            "From: {from}\r\nTo: me@x.test\r\nSubject: Pics {id}\r\nMessage-ID: <{id}@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\
             Content-Type: text/html\r\n\r\n<p>Hi <img src=\"https://img.test/{id}.png\"></p>\r\n"
        )
        .into_bytes()
    };
    deliver(&fake, "a1", html("a1", "dave@z.test"), "INBOX");
    deliver(&fake, "a2", html("a2", "dave@z.test"), "Junk");
    deliver(&fake, "a3", html("a3", "spammer@y.test"), "INBOX");
    sync_until(&engine, &account, "a3").await;
    let inbox = get(&engine, &account, "a1").await;
    assert!(inbox.remote_images_allowed && inbox.html_body.unwrap().contains("img.test/a1.png"));
    for id in ["a2", "a3"] {
        let spam = get(&engine, &account, id).await;
        assert!(!spam.remote_images_allowed && spam.remote_images_blocked, "{id}: spam stays blocked");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_organizer_cant_act_on_or_remove_a_real_meeting() {
    let (engine, fake, account, _dir) = setup().await;
    // Bob's real meeting is on the synced calendar.
    let real = invite_ics("meet-9", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:bob@x.test").replace("Claim your prize", "Board meeting");
    fake.events.lock().unwrap().push(("e9".into(), real));
    // A forged copy from "Bob" whose DMARC check failed.
    let spoof_ics = invite_ics("meet-9", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:bob@x.test").replace("Claim your prize", "Cancelled, ignore");
    let spoof = String::from_utf8(invite_mail("g1", &spoof_ics)).unwrap().replace("From: Prize <noreply@prize.test>", "From: Bob <bob@x.test>");
    deliver(&fake, "g1", with_header(spoof.into_bytes(), "Authentication-Results: mx.google.com; spf=fail smtp.mailfrom=x.test; dmarc=fail header.from=x.test"), "INBOX");
    // Another sender's invitation naming Bob as organizer, honestly sent.
    deliver(&fake, "g2", invite_mail("g2", &invite_ics("meet-9", "NEEDS-ACTION").replace("ORGANIZER:mailto:noreply@prize.test", "ORGANIZER:mailto:bob@x.test")), "INBOX");
    sync_until(&engine, &account, "g2").await;
    wait_for(|| !engine.store().events_by_uid(&account, "meet-9").unwrap().is_empty()).await;

    let m = get(&engine, &account, "g1").await;
    assert_eq!(m.invite.as_ref().unwrap().event_id, None, "a forged invitation isn't shown as Bob's meeting");
    let err = engine.handle(&Caller::User, Command::RespondToEmailInvite { message_id: m.summary.id.clone(), response: PartStat::Declined }).await.unwrap_err();
    assert!(err.message.contains("sender checks"), "{}", err.message);
    // Marking either as spam leaves Bob's meeting alone.
    let ids = vec![m.summary.id, get(&engine, &account, "g2").await.summary.id];
    let r = engine.handle(&Caller::User, Command::MarkSpam { message_ids: ids }).await.unwrap();
    assert_eq!(r["invites_removed"], serde_json::json!(0));
    let stored = engine.store().events_by_uid(&account, "meet-9").unwrap();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].ics.contains("PARTSTAT=NEEDS-ACTION") && stored[0].ics.contains("Board meeting"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_servers_authserv_id_is_learned_from_recent_mail() {
    let (engine, fake, account, _dir) = setup().await;
    // An IMAP account on a server Satchel has no built-in id for.
    let config: AccountConfig = serde_json::from_value(json!({
        "kind": "imap", "email": "me@x.test", "display_name": "Me", "username": "me", "imap_host": "mail.x.test", "imap_port": 993,
        "imap_security": "tls", "smtp_host": "mail.x.test", "smtp_port": 465, "smtp_security": "tls"
    }))
    .unwrap();
    engine.store().update_account_config(&account, &config).unwrap();
    let mut s = engine.settings();
    s.remote_image_senders = vec!["carol@x.test".into()];
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    let mail = |id: &str, from: &str| {
        let raw = format!(
            "From: {from}\r\nTo: me@x.test\r\nSubject: Note {id}\r\nMessage-ID: <{id}@x.test>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0000\r\n\
             Content-Type: text/html\r\n\r\n<p>Hi <img src=\"https://img.x.test/{id}.png\"></p>\r\n"
        );
        let domain = from.rsplit('@').next().unwrap();
        with_header(raw.into_bytes(), &format!("Authentication-Results: MX.X.test; spf=pass smtp.mailfrom={domain}; dkim=pass header.d={domain}; dmarc=pass header.from={domain}"))
    };
    deliver(&fake, "l0", mail("l0", "carol@x.test"), "INBOX");
    sync_until(&engine, &account, "l0").await;
    assert!(get(&engine, &account, "l0").await.remote_images_blocked, "nothing is trusted before the server's id is learned");
    for i in 1..=20 {
        deliver(&fake, &format!("l{i}"), mail(&format!("l{i}"), &format!("n{i}@site{i}.test")), "INBOX");
    }
    sync_until(&engine, &account, "l20").await;
    let learned = get(&engine, &account, "l0").await;
    assert!(learned.remote_images_allowed && !learned.remote_images_blocked, "the id on most recent mail is trusted");
    // Another server's header still isn't.
    let forged = String::from_utf8(mail("l21", "carol@x.test")).unwrap().replacen("MX.X.test", "mx.evil.test", 1);
    deliver(&fake, "l21", forged.into_bytes(), "INBOX");
    sync_until(&engine, &account, "l21").await;
    assert!(get(&engine, &account, "l21").await.remote_images_blocked);
}
