use super::*;

#[test]
fn rebases_loopback_only() {
    assert_eq!(rebase("https://localhost/jmap/", "http://127.0.0.1:8480/jmap/session"), "http://127.0.0.1:8480/jmap/");
    assert_eq!(rebase("https://api.fastmail.com/jmap/api/", "https://api.fastmail.com/jmap/session"), "https://api.fastmail.com/jmap/api/");
    assert_eq!(rebase("https://localhost/jmap/download/{accountId}/{blobId}/{name}?accept={type}", "http://127.0.0.1:8480/s"), "http://127.0.0.1:8480/jmap/download/{accountId}/{blobId}/{name}?accept={type}");
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
