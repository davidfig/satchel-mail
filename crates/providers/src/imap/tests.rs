use super::*;

#[test]
fn mutf7() {
    assert_eq!(decode_mutf7("INBOX"), "INBOX");
    assert_eq!(decode_mutf7("&ZeVnLIqe-"), "日本語");
    assert_eq!(decode_mutf7("Caf&AOk-"), "Café");
    assert_eq!(decode_mutf7("A&-B"), "A&B");
    assert_eq!(encode_mutf7("INBOX"), "INBOX");
    assert_eq!(encode_mutf7("日本語"), "&ZeVnLIqe-");
    assert_eq!(encode_mutf7("Café"), "Caf&AOk-");
    assert_eq!(encode_mutf7("A&B"), "A&-B");
    for s in ["Reisen 🌍 2026", "Ünïcödé/ok", "~/,&"] {
        assert_eq!(decode_mutf7(&encode_mutf7(s)), s);
    }
}

#[test]
fn uidsets() {
    assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10]), "1:3,5,9:10");
}

/// Needs `scripts/dev-servers start`; run with `cargo test -p satchel-providers -- --ignored`.
#[tokio::test]
#[ignore]
async fn expunge_without_uidplus_keeps_other_clients_deletions() {
    let creds = CredentialHandle::new("expunge-test".into(), std::sync::Arc::new(satchel_core::secrets::MemSecretStore::default()));
    creds.set(&Credential::Password { password: "test-pass-carol".into() }).unwrap();
    let cfg = ImapSettings {
        host: "127.0.0.1".into(),
        port: 2993,
        security: Security::Tls,
        username: "carol".into(),
        smtp_host: "127.0.0.1".into(),
        smtp_port: 2465,
        smtp_security: Security::Tls,
        allow_invalid_certs: true,
    };
    let p = ImapProvider::new(cfg, creds);
    let mut s = p.connect().await.unwrap();
    p.caps.lock().await.remove("UIDPLUS");

    let mb = "ExpungeTest";
    let _ = s.delete(mb).await;
    s.create(mb).await.unwrap();
    for subject in ["theirs", "ours"] {
        let raw = format!("From: a@satchel.test\r\nSubject: {subject}\r\n\r\nbody\r\n");
        s.append(mb, None, None, raw.as_bytes()).await.unwrap();
    }
    s.select(mb).await.unwrap();
    let mut all: Vec<u32> = s.uid_search("ALL").await.unwrap().into_iter().collect();
    all.sort_unstable();
    let (theirs, ours) = (all[0], all[1]);
    let _: Vec<Fetch> = s.uid_store(theirs.to_string(), "+FLAGS.SILENT (\\Deleted)").await.unwrap().try_collect().await.unwrap();

    p.expunge_uids(&mut s, mb, &[ours]).await.unwrap();

    let left: Vec<u32> = s.uid_search("ALL").await.unwrap().into_iter().collect();
    let deleted: Vec<u32> = s.uid_search("DELETED").await.unwrap().into_iter().collect();
    s.close().await.unwrap();
    s.delete(mb).await.unwrap();
    assert_eq!(left, vec![theirs]);
    assert_eq!(deleted, vec![theirs]);
}

#[test]
fn remote_ids() {
    let r = remote_id(7, 42, "Folder:With:Colons");
    assert_eq!(parse_remote_id(&r), Some((7, 42, "Folder:With:Colons")));
}
