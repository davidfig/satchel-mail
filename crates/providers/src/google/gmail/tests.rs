use super::*;

#[test]
fn metadata_to_record() {
    let m = json!({
        "id": "18a", "threadId": "18t", "labelIds": ["INBOX", "UNREAD", "Label_1"], "snippet": "Hi &amp; bye", "internalDate": "1767225600000", "sizeEstimate": 1234,
        "payload": { "headers": [
            { "name": "From", "value": "Bob <bob@x.test>" }, { "name": "To", "value": "a@x.test" }, { "name": "Subject", "value": "Hello" },
            { "name": "Message-ID", "value": "<m1@x>" }, { "name": "In-Reply-To", "value": "<m0@x>" }
        ] }
    });
    let r = record_from_metadata(&m);
    assert_eq!(r.subject, "Hello");
    assert_eq!(r.from.unwrap().email, "bob@x.test");
    assert_eq!(r.message_id_hdr.as_deref(), Some("m1@x"));
    assert_eq!(r.in_reply_to.as_deref(), Some("m0@x"));
    assert!(!r.flags.seen);
    assert_eq!(r.snippet, "Hi & bye");
    assert_eq!(r.remote_thread_id.as_deref(), Some("18t"));
}

/// `p` as Gmail's `format=full` describes it: text inline, other contents
/// behind an attachment id.
fn gmail_part(p: &crate::test_mail::TestPart, n: &mut usize, atts: &mut HashMap<String, Vec<u8>>) -> Value {
    let headers: Vec<Value> = p.fields().into_iter().map(|(n, v)| json!({ "name": n, "value": v })).collect();
    let mime = p.mime_type();
    if mime.starts_with("multipart/") {
        let parts: Vec<Value> = p.children.iter().map(|c| gmail_part(c, n, atts)).collect();
        return json!({ "mimeType": mime, "filename": "", "headers": headers, "body": { "size": 0 }, "parts": parts });
    }
    let data = p.decoded();
    let body = if mime.starts_with("text/") {
        json!({ "size": data.len(), "data": b64url(&data) })
    } else {
        *n += 1;
        let id = format!("att-{n}");
        let b = json!({ "size": data.len(), "attachmentId": id });
        atts.insert(id, data);
        b
    };
    json!({ "mimeType": mime, "filename": "", "headers": headers, "body": body })
}

#[test]
fn downloads_without_attachments() {
    let raw = crate::test_mail::sample();
    let mut atts = HashMap::new();
    let payload = gmail_part(&crate::test_mail::split(&raw), &mut 0, &mut atts);
    assert!(parts::skips_any(&payload, false));
    let mut wanted = vec![];
    parts::to_fetch(&payload, false, &mut wanted);
    // Only the inline logo is fetched, not the PDF.
    assert_eq!(wanted.len(), 1);
    assert_eq!(atts[&wanted[0]], b"\x89PNG\r\n\x1a\n");
    let fetched: HashMap<String, Vec<u8>> = wanted.iter().map(|w| (w.clone(), atts[w].clone())).collect();
    let slim = satchel_mime::rebuild(&parts::part(&payload, false, &fetched).unwrap()).unwrap();
    crate::test_mail::assert_slim(&raw, &slim.raw, &slim.attachments);
    assert_eq!(slim.attachments.iter().find(|a| a.filename == "report.pdf").unwrap().size, crate::test_mail::pdf_size());
    // Plain mail has nothing to leave out.
    let plain = json!({ "mimeType": "text/plain", "headers": [], "body": { "size": 5, "data": b64url(b"hello") } });
    assert!(!parts::skips_any(&plain, false));
}
