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

/// A tiny IMAP server for the watcher: signs in (or refuses), opens the inbox
/// with `exists` messages, and sends `event` during IDLE. Returns the port
/// and, once the client leaves, the commands it sent.
async fn fake_imap(exists: u32, event: Option<&'static str>, refuse: bool) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        let mut seen = vec![];
        write.write_all(b"* OK ready\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let (tag, cmd) = line.split_once(' ').unwrap_or(("", &line));
            let (tag, cmd) = (tag.to_string(), cmd.to_string());
            seen.push(cmd.clone());
            if cmd.starts_with("AUTHENTICATE") {
                write.write_all(b"+ \r\n").await.unwrap();
                let answer = lines.next_line().await.unwrap().unwrap();
                seen.push(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(answer).unwrap()).unwrap());
                if refuse {
                    // Google's error challenge, which the client must answer empty.
                    write.write_all(b"+ eyJzdGF0dXMiOiI0MDAifQ==\r\n").await.unwrap();
                    seen.push(lines.next_line().await.unwrap().unwrap());
                    write.write_all(format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n").as_bytes()).await.unwrap();
                } else {
                    write.write_all(format!("{tag} OK signed in\r\n").as_bytes()).await.unwrap();
                }
            } else if cmd.starts_with("EXAMINE") {
                write.write_all(format!("* {exists} EXISTS\r\n{tag} OK [READ-ONLY] opened\r\n").as_bytes()).await.unwrap();
            } else if cmd == "IDLE" {
                write.write_all(b"+ idling\r\n").await.unwrap();
                if let Some(event) = event {
                    write.write_all(format!("{event}\r\n").as_bytes()).await.unwrap();
                }
                let done = lines.next_line().await.unwrap().unwrap();
                seen.push(done);
                write.write_all(format!("{tag} OK idle done\r\n").as_bytes()).await.unwrap();
            } else if cmd == "LOGOUT" {
                write.write_all(format!("* BYE\r\n{tag} OK bye\r\n").as_bytes()).await.unwrap();
                break;
            }
        }
        seen
    });
    (port, task)
}

async fn fake_client(port: u16) -> crate::imap::Client {
    crate::imap::open_client("127.0.0.1", port, Security::None, false, None).await.unwrap()
}

#[tokio::test]
async fn watcher_signs_in_read_only_and_wakes_on_new_mail() {
    let (port, server) = fake_imap(5, Some("* 6 EXISTS"), false).await;
    let (session, exists) = watch::open_inbox(fake_client(port).await, "me@x.test", "tok").await.unwrap();
    assert_eq!(exists, 5);
    let last = std::sync::Mutex::new(Some(5));
    let started = std::time::Instant::now();
    assert!(watch::wait_on_inbox(session, exists, &last, std::time::Duration::from_secs(30)).await.unwrap());
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let seen = server.await.unwrap();
    assert_eq!(seen[0], "AUTHENTICATE XOAUTH2");
    assert_eq!(seen[1], "user=me@x.test\x01auth=Bearer tok\x01\x01");
    assert!(seen.iter().any(|c| c == "EXAMINE \"INBOX\""), "{seen:?}");
    assert!(!seen.iter().any(|c| c.starts_with("SELECT")), "{seen:?}");
    assert!(seen.iter().any(|c| c == "DONE"), "{seen:?}");
}

#[tokio::test]
async fn watcher_keeps_waiting_through_keepalives_until_the_deadline() {
    let (port, server) = fake_imap(5, Some("* OK Still here"), false).await;
    let (session, exists) = watch::open_inbox(fake_client(port).await, "me@x.test", "tok").await.unwrap();
    let last = std::sync::Mutex::new(None);
    assert!(!watch::wait_on_inbox(session, exists, &last, std::time::Duration::from_millis(200)).await.unwrap());
    assert_eq!(*last.lock().unwrap(), Some(5));
    server.await.unwrap();
}

#[tokio::test]
async fn watcher_returns_at_once_for_mail_that_came_between_waits() {
    let (port, server) = fake_imap(6, None, false).await;
    let (session, exists) = watch::open_inbox(fake_client(port).await, "me@x.test", "tok").await.unwrap();
    let last = std::sync::Mutex::new(Some(5));
    assert!(watch::wait_on_inbox(session, exists, &last, std::time::Duration::from_secs(30)).await.unwrap());
    assert_eq!(*last.lock().unwrap(), Some(6));
    let seen = server.await.unwrap();
    assert!(!seen.iter().any(|c| c == "IDLE"), "{seen:?}");
}

#[tokio::test]
async fn watcher_answers_a_refused_sign_in_without_resending_the_token() {
    let (port, server) = fake_imap(5, None, true).await;
    assert!(watch::open_inbox(fake_client(port).await, "me@x.test", "tok").await.is_err());
    let seen = server.await.unwrap();
    assert_eq!(seen[2], "", "{seen:?}");
}

#[tokio::test]
async fn a_failed_watch_falls_back_to_waiting_out_the_poll() {
    let wait = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    watch::or_poll(async { anyhow::bail!("IMAP is turned off") }, wait).await;
    assert!(started.elapsed() >= wait);
    // A watch that saw mail doesn't wait.
    let started = std::time::Instant::now();
    watch::or_poll(async { Ok(true) }, std::time::Duration::from_secs(30)).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
