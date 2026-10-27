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

#[test]
fn strip_attachments_keeps_text_and_attachment_list() {
    let pdf = "QUFB".repeat(20_000);
    let raw = format!(
        "From: Bob <bob@x.test>\r\nSubject: Report\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: multipart/related; boundary=\"b2\"\r\n\r\n--b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>See <img src=\"cid:logo\"> attached</p>\r\n--b2\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b2--\r\n--b1\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b1\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b1--\r\n"
    );
    let slim = strip_attachments(raw.as_bytes()).expect("stripped");
    assert!(slim.len() < 1000, "{} bytes", slim.len());
    let (a, b) = (parse(raw.as_bytes()).unwrap(), parse(&slim).unwrap());
    assert_eq!(a.html, b.html);
    assert_eq!(b.calendar_parts.len(), 1);
    let names = |p: &Parsed| p.attachments.iter().map(|a| (a.id.clone(), a.filename.clone())).collect::<Vec<_>>();
    assert_eq!(names(&a), names(&b));
    // The small inline logo stays; the PDF's contents are gone.
    assert!(!inline_data_urls(&slim).is_empty());
    let pdf_id = &a.attachments.iter().find(|x| x.filename == "report.pdf").unwrap().id;
    assert!(attachment_bytes(&slim, pdf_id).unwrap().2.is_empty());
    // Nothing to strip in plain mail.
    assert!(strip_attachments(b"Subject: Hi\r\n\r\nHello\r\n").is_none());
}

/// The part tree a server would describe for `raw`, downloading only what
/// [`keeps_contents`] asks for.
fn tree(raw: &[u8], msg: &mail_parser::Message, i: usize, in_alt: bool) -> Part {
    let p = &msg.parts[i];
    let mut headers = raw[p.offset_header as usize..p.offset_body as usize].to_vec();
    while headers.ends_with(b"\r\n\r\n") {
        headers.truncate(headers.len() - 2);
    }
    let ct = p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or(""))).unwrap_or_else(|| "text/plain".into());
    let body = match &p.body {
        mail_parser::PartType::Multipart(ids) => Body::Multipart(ids.iter().map(|id| tree(raw, msg, *id as usize, ct == "multipart/alternative")).collect()),
        _ => {
            let disp = p.content_disposition().map(|d| d.ctype().to_string());
            if keeps_contents(&ct, disp.as_deref(), p.content_id().is_some(), p.contents().len() as u64, in_alt) {
                Body::Encoded(raw[p.offset_body as usize..p.offset_end as usize].to_vec())
            } else {
                Body::Skipped(p.contents().len() as u64)
            }
        }
    };
    Part { headers, body }
}

#[test]
fn rebuild_matches_stripped_copy() {
    let pdf = "QUFB".repeat(20_000);
    let raw = format!(
        "From: Bob <bob@x.test>\r\nSubject: Report\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\npreamble\r\n--b1\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n--b2\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=E9 report\r\n--b2\r\nContent-Type: multipart/related; boundary=\"b3\"\r\n\r\n--b3\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>See <img src=\"cid:logo\"> attached</p>\r\n--b3\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b3--\r\n--b2--\r\n--b1\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b1\r\nContent-Type: text/csv; name=\"data.csv\"\r\nContent-Disposition: attachment; filename=\"data.csv\"\r\n\r\na,b\r\n1,2\r\n--b1\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b1--\r\n"
    );
    let raw = raw.as_bytes();
    let msg = MessageParser::default().parse(raw).unwrap();
    let root = tree(raw, &msg, 0, false);
    let slim = rebuild(&root).expect("rebuilt");
    let (full, stripped, rebuilt) = (parse(raw).unwrap(), parse(&strip_attachments(raw).unwrap()).unwrap(), parse(&slim.raw).unwrap());
    assert!(slim.raw.len() < 2000, "{} bytes", slim.raw.len());
    assert_eq!((&rebuilt.text, &rebuilt.html, &rebuilt.calendar_parts), (&stripped.text, &stripped.html, &stripped.calendar_parts));
    assert_eq!(rebuilt.text, full.text);
    let ids = |p: &[Attachment]| p.iter().map(|a| (a.id.clone(), a.filename.clone(), a.mime_type.clone(), a.inline, a.size)).collect::<Vec<_>>();
    assert_eq!(ids(&slim.attachments), ids(&full.attachments));
    assert!(!inline_data_urls(&slim.raw).is_empty());
    // A part decoded by the server is written back out as base64.
    let mut root = root;
    let Body::Multipart(parts) = &mut root.body else { panic!() };
    let Body::Multipart(alt) = &mut parts[0].body else { panic!() };
    alt[0] = Part { headers: b"Content-Type: text/plain; charset=iso-8859-1; format=flowed\r\nContent-Transfer-Encoding: quoted-printable\r\n".to_vec(), body: Body::Text("Caf\u{e9} report".into()) };
    let again = parse(&rebuild(&root).unwrap().raw).unwrap();
    assert_eq!(again.text, full.text);
    assert!(rebuild(&Part { headers: b"Content-Type: multipart/mixed\r\n".to_vec(), body: Body::Multipart(vec![]) }).is_none());
}
