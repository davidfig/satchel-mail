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

#[test]
fn downloads_without_attachments() {
    use parts::{Plan, Section};
    let raw = crate::test_mail::sample();
    // Sections by IMAP part number, as a server would send them.
    let tree = crate::test_mail::split(&raw);
    let mut sections: HashMap<Section, Vec<u8>> = HashMap::from([(Section::Header, tree.headers.clone())]);
    fn add(p: &crate::test_mail::TestPart, path: Vec<u32>, out: &mut HashMap<Section, Vec<u8>>) {
        for (i, c) in p.children.iter().enumerate() {
            let sub: Vec<u32> = path.iter().copied().chain([i as u32 + 1]).collect();
            out.insert(Section::Mime(sub.clone()), c.headers.clone());
            out.insert(Section::Body(sub.clone()), c.body.clone());
            add(c, sub, out);
        }
    }
    add(&tree, vec![], &mut sections);
    let pdf_octets = sections[&Section::Body(vec![3])].len();
    let response = format!(
        "* 1 FETCH (UID 7 BODYSTRUCTURE (((\"TEXT\" \"PLAIN\" (\"CHARSET\" \"iso-8859-1\") NIL NIL \"QUOTED-PRINTABLE\" 13 1)(\"TEXT\" \"HTML\" (\"CHARSET\" \"utf-8\") NIL NIL \"7BIT\" 30 1) \"ALTERNATIVE\" (\"BOUNDARY\" \"b2\") NIL NIL)\
         (\"IMAGE\" \"PNG\" NIL \"<logo>\" NIL \"BASE64\" 12 NIL (\"INLINE\" NIL) NIL)\
         (\"APPLICATION\" \"PDF\" (\"NAME\" \"report.pdf\") NIL NIL \"BASE64\" {pdf_octets} NIL (\"ATTACHMENT\" (\"FILENAME\" \"report.pdf\")) NIL)\
         (\"TEXT\" \"CALENDAR\" (\"CHARSET\" \"utf-8\" \"METHOD\" \"REQUEST\") NIL NIL \"7BIT\" 31 2) \"MIXED\" (\"BOUNDARY\" \"b1\") NIL NIL))\r\n"
    );
    let (_, parsed) = async_imap::imap_proto::Response::parse(response.as_bytes()).unwrap();
    let async_imap::imap_proto::Response::Fetch(_, attrs) = parsed else { panic!("not a FETCH") };
    let bs = attrs.iter().find_map(|a| match a { async_imap::imap_proto::AttributeValue::BodyStructure(b) => Some(b), _ => None }).unwrap();
    let plan = Plan::new(bs);
    assert!(plan.skips_any());
    assert_eq!(
        plan.fetch_items(),
        "(UID BODY.PEEK[HEADER] BODY.PEEK[1.MIME] BODY.PEEK[1.1.MIME] BODY.PEEK[1.1] BODY.PEEK[1.2.MIME] BODY.PEEK[1.2] BODY.PEEK[2.MIME] BODY.PEEK[2] BODY.PEEK[3.MIME] BODY.PEEK[4.MIME] BODY.PEEK[4])"
    );
    let part = plan.part(&|s| sections.get(s).cloned()).unwrap();
    let slim = satchel_mime::rebuild(&part).unwrap();
    crate::test_mail::assert_slim(&raw, &slim.raw, &slim.attachments);
    let pdf = slim.attachments.iter().find(|a| a.filename == "report.pdf").unwrap();
    assert_eq!(pdf.size, crate::test_mail::pdf_size());
}

/// Needs `scripts/dev-servers start`; run with `cargo test -p satchel-providers -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_download_without_attachments() {
    let creds = CredentialHandle::new("slim-test".into(), std::sync::Arc::new(satchel_core::secrets::MemSecretStore::default()));
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
    let mb = "SlimTest";
    let _ = s.delete(mb).await;
    s.create(mb).await.unwrap();
    let raw = crate::test_mail::sample();
    s.append(mb, None, None, &raw).await.unwrap();
    s.select(mb).await.unwrap();
    let uid = s.uid_search("ALL").await.unwrap().into_iter().next().unwrap();
    let plan = p.plans(&mut s, &uid.to_string()).await.unwrap().remove(&uid).expect("has attachments to leave out");
    let slim = p.fetch_slim(&mut s, uid, &plan).await.unwrap().expect("rebuilt");
    let full = ImapProvider::fetch_full(&mut s, uid).await.unwrap();
    s.close().await.unwrap();
    s.delete(mb).await.unwrap();
    assert_eq!(full, raw);
    crate::test_mail::assert_slim(&raw, &slim.raw, &slim.attachments);
    let pdf = slim.attachments.iter().find(|a| a.filename == "report.pdf").unwrap();
    assert_eq!(pdf.size, crate::test_mail::pdf_size());
}
