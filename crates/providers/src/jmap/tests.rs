use super::*;

#[test]
fn rebases_loopback_only() {
    assert_eq!(rebase("https://localhost/jmap/", "http://127.0.0.1:8480/jmap/session"), "http://127.0.0.1:8480/jmap/");
    assert_eq!(rebase("https://api.fastmail.com/jmap/api/", "https://api.fastmail.com/jmap/session"), "https://api.fastmail.com/jmap/api/");
    assert_eq!(rebase("https://localhost/jmap/download/{accountId}/{blobId}/{name}?accept={type}", "http://127.0.0.1:8480/s"), "http://127.0.0.1:8480/jmap/download/{accountId}/{blobId}/{name}?accept={type}");
}

#[test]
fn session_urls_keep_credentials_safe() {
    let (api, dl) = (Use::Api, Use::Download);
    let s = "https://api.fastmail.com/jmap/session";
    assert!(endpoint("https://api.fastmail.com/jmap/api/", s, api).is_ok());
    assert!(endpoint("https://api.fastmail.com/jmap/event/", s, api).is_ok());
    let fm_download = "https://www.fastmailusercontent.com/jmap/download/{accountId}/{blobId}/{name}?type={type}";
    assert!(endpoint(fm_download, s, dl).is_ok());
    assert!(endpoint(fm_download, s, api).is_err(), "only downloads may leave the site");
    assert!(endpoint("https://evilfastmailusercontent.com/x", s, dl).is_err());
    assert!(endpoint("http://www.fastmailusercontent.com/x", s, dl).is_err(), "no downgrade");
    assert!(endpoint("http://api.fastmail.com/jmap/api/", s, api).is_err(), "no downgrade");
    assert!(endpoint("https://evil.test/jmap/api/", s, api).is_err());
    assert!(endpoint("https://evil.test/jmap/download/", s, dl).is_err());
    // Same site, like DAV: another host of the session's domain.
    assert!(endpoint("https://jmap2.example.com/api/", "https://mail.example.com/.well-known/jmap", api).is_ok());
    // A public server may not point us at this computer or a private network.
    for internal in ["https://10.0.0.1/jmap/", "https://localhost:8443/jmap/", "https://[::1]/jmap/", "https://169.254.169.254/latest/"] {
        assert!(endpoint(internal, s, api).is_err(), "{internal}");
        assert!(endpoint(internal, s, dl).is_err(), "{internal}");
    }
    let local = "http://127.0.0.1:8480/jmap/session";
    assert!(endpoint(&rebase("https://localhost/jmap/", local), local, api).is_ok());
    assert!(endpoint(&rebase("https://localhost/jmap/download/{accountId}/{blobId}/{name}", local), local, dl).is_ok());
    assert!(endpoint("http://evil.test/jmap/", local, api).is_err());
    assert!(endpoint("https://evil.test/jmap/", local, api).is_err());
    assert!(endpoint("https://www.fastmailusercontent.com/x", local, dl).is_err(), "no known hosts from a plain-http session");
}

#[test]
fn converts_email() {
    let e = json!({ "id": "M1", "threadId": "T1", "mailboxIds": { "a": true, "b": false }, "keywords": { "$seen": true },
        "receivedAt": "2026-01-02T03:04:05Z", "messageId": ["x@y"], "subject": "Hi", "from": [{ "name": "Bob", "email": "b@x" }], "preview": "hello" });
    let r = record(&e);
    assert_eq!(r.remote_thread_id.as_deref(), Some("T1"));
    assert_eq!(r.message_id_hdr.as_deref(), Some("x@y"));
    assert!(r.flags.seen && !r.flags.flagged);
    assert_eq!(mailboxes_of(&e), vec![("a".to_string(), None)]);
}

/// `p` as JMAP's `bodyStructure` describes it, with text in `values` and
/// every leaf's contents in `blobs`.
fn jmap_part(p: &crate::test_mail::TestPart, n: &mut usize, values: &mut serde_json::Map<String, Value>, blobs: &mut HashMap<String, Vec<u8>>) -> Value {
    // Raw header values keep the space after the colon.
    let headers: Vec<Value> = p.fields().into_iter().map(|(n, v)| json!({ "name": n, "value": format!(" {v}") })).collect();
    let mime = p.mime_type();
    if mime.starts_with("multipart/") {
        let subs: Vec<Value> = p.children.iter().map(|c| jmap_part(c, n, values, blobs)).collect();
        return json!({ "partId": null, "type": mime, "headers": headers, "size": 0, "subParts": subs });
    }
    *n += 1;
    let (id, blob) = (n.to_string(), format!("B{n}"));
    let data = p.decoded();
    if mime == "text/plain" || mime == "text/html" {
        // The server decodes text from its charset.
        let text: String = if p.header("Content-Type").unwrap().contains("iso-8859-1") { data.iter().map(|b| *b as char).collect() } else { String::from_utf8(data.clone()).unwrap() };
        values.insert(id.clone(), json!({ "value": text, "isTruncated": false, "isEncodingProblem": false }));
    }
    let disposition = p.header("Content-Disposition").map(|d| d.split(';').next().unwrap().trim().to_string());
    let cid = p.header("Content-ID").map(|c| c.trim_matches(['<', '>']).to_string());
    blobs.insert(blob.clone(), data.clone());
    json!({ "partId": id, "blobId": blob, "type": mime, "headers": headers, "size": data.len(), "disposition": disposition, "cid": cid })
}

#[test]
fn downloads_without_attachments() {
    let raw = crate::test_mail::sample();
    let (mut values, mut blobs) = (serde_json::Map::new(), HashMap::new());
    let root = jmap_part(&crate::test_mail::split(&raw), &mut 0, &mut values, &mut blobs);
    let values = Value::Object(values);
    assert!(parts::skips_any(&root, false));
    let mut wanted = vec![];
    parts::to_fetch(&root, false, &values, &mut wanted);
    // The logo and the invitation download; the text came as values, the PDF stays.
    assert_eq!(wanted.len(), 2);
    let fetched: HashMap<String, Vec<u8>> = wanted.iter().map(|w| (w.clone(), blobs[w].clone())).collect();
    let slim = satchel_mime::rebuild(&parts::part(&root, false, &values, &fetched).unwrap()).unwrap();
    crate::test_mail::assert_slim(&raw, &slim.raw, &slim.attachments);
    assert_eq!(slim.attachments.iter().find(|a| a.filename == "report.pdf").unwrap().size, crate::test_mail::pdf_size());
}

/// Needs `scripts/dev-servers start`; run with `cargo test -p satchel-providers -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_download_without_attachments() {
    let creds = CredentialHandle::new("slim-test".into(), std::sync::Arc::new(satchel_core::secrets::MemSecretStore::default()));
    creds.set(&Credential::Password { password: "test-pass-carol".into() }).unwrap();
    let p = JmapProvider::new("http://127.0.0.1:8480/.well-known/jmap".into(), "carol".into(), true, creds).unwrap();
    let acc = p.session().await.unwrap().account_id;
    let (_, roles, _) = p.mailboxes(&acc).await.unwrap();
    let raw = crate::test_mail::sample();
    let id = p.import(&acc, &raw, &roles[&MailboxRole::Inbox], json!({})).await.unwrap();
    let slim = p.fetch_slim(&id).await.unwrap();
    p.call(vec![json!(["Email/set", { "accountId": acc, "destroy": [id] }, "d"])]).await.unwrap();
    let slim = slim.expect("rebuilt");
    crate::test_mail::assert_slim(&raw, &slim.raw, &slim.attachments);
    let pdf = slim.attachments.iter().find(|a| a.filename == "report.pdf").unwrap();
    assert_eq!(pdf.size, crate::test_mail::pdf_size());
}
