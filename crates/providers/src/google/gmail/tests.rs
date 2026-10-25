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
