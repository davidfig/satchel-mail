use super::*;

#[test]
fn parse_basic() {
    let raw = b"From: Bob <bob@x.test>\r\nTo: a@x.test, \"C\" <c@x.test>\r\nSubject: Hi\r\nMessage-ID: <1@x>\r\nIn-Reply-To: <0@x>\r\nReferences: <a@x> <0@x>\r\nDate: Tue, 1 Jul 2025 10:00:00 +0000\r\n\r\nHello\r\n\r\nOn Mon, X wrote:\r\n> old\r\n";
    let p = parse(raw).unwrap();
    assert_eq!(p.subject, "Hi");
    assert_eq!(p.from.unwrap().email, "bob@x.test");
    assert_eq!(p.to.len(), 2);
    assert_eq!(p.message_id.as_deref(), Some("1@x"));
    assert_eq!(p.in_reply_to.as_deref(), Some("0@x"));
    assert_eq!(p.references, vec!["a@x", "0@x"]);
    assert_eq!(p.snippet, "Hello");
    assert!(p.html.is_none());
}

#[test]
fn html_only_mail_keeps_paragraphs_and_links() {
    let raw = b"From: Carol <carol@x.test>\r\nSubject: Notes\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<div>Hi Alice,</div><div><br></div><div>Here is <b>bold</b> and <a href=\"https://x.test/notes\">the notes</a>.</div><div><br></div><div>Thanks,<br>Carol</div>";
    let p = parse(raw).unwrap();
    assert!(p.html.is_some());
    assert_eq!(p.text, "Hi Alice,\n\nHere is bold and the notes (https://x.test/notes).\n\nThanks,\nCarol");
}

#[test]
fn html_to_text_links_and_wrapping() {
    let long = "word ".repeat(40);
    let t = html_to_text(&format!(
        r##"<p>{long}</p><p><a href="https://x.test/a">https://x.test/a</a>, <a href="mailto:a@x.test">a@x.test</a>, <a href="mailto:a@x.test">mail me</a>, <a href="#top">top</a></p><p><img src="cid:logo" alt="Logo"></p>"##
    ));
    assert_eq!(t, format!("{}\n\nhttps://x.test/a, a@x.test, mail me (a@x.test), top", long.trim_end()));
    assert_eq!(html_to_text(r#"<img src="https://x.test/banner.png" alt="Banner">"#), "");
}

#[test]
fn sanitize_blocks_scripts_and_remote() {
    let s = sanitize_html(
        r#"<p onclick="x()">hi</p><script>alert(1)</script><img src="https://t.example/p.gif"><img src="cid:abc"><div style="background:url(https://x/y.png)">z</div>"#,
        false,
        &[("abc".into(), "data:image/png;base64,AAAA".into())],
    );
    assert!(!s.html.contains("script"));
    assert!(!s.html.contains("onclick"));
    assert!(!s.html.contains("t.example"));
    assert!(s.html.contains("data:image/png;base64,AAAA"));
    assert!(!s.html.contains("x/y.png"));
    assert!(s.had_remote_content);
}

#[test]
fn sanitize_keeps_quote_markers() {
    let s = sanitize_html(r#"<p>new</p><div id="divRplyFwdMsg">From: a</div><blockquote type="cite">old</blockquote>"#, false, &[]);
    assert!(s.html.contains(r#"id="divRplyFwdMsg""#));
    assert!(s.html.contains(r#"type="cite""#));
}

#[test]
fn build_roundtrip() {
    let o = Outgoing {
        from: Address { name: Some("Alice".into()), email: "alice@x.test".into() },
        to: vec![Address::new("bob@x.test")],
        subject: "Re: Test ✓".into(),
        text: "Body".into(),
        in_reply_to: Some("0@x".into()),
        references: vec!["0@x".into()],
        message_id: "m1@x.test".into(),
        date: Utc::now(),
        attachments: vec![("a.txt".into(), "text/plain".into(), b"hello".to_vec())],
        ..Default::default()
    };
    let raw = build(&o);
    let p = parse(&raw).unwrap();
    assert_eq!(p.subject, "Re: Test ✓");
    assert_eq!(p.in_reply_to.as_deref(), Some("0@x"));
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(attachment_bytes(&raw, "0").unwrap().2, b"hello");
}

#[test]
fn subjects() {
    assert_eq!(normalize_subject("Re: RE: Fwd: hello"), "hello");
    assert_eq!(reply_subject("Re: hello"), "Re: hello");
}
