// SPDX-License-Identifier: MIT OR Apache-2.0
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
fn html_to_text_blocks_lists_and_quotes() {
    // A whitespace-only list once crashed the old converter and stopped sync.
    assert_eq!(html_to_text("<p>Rate us</p><ul> </ul><ul><li>one</li></ul>"), "Rate us\n\n• one");
    assert_eq!(
        html_to_text("<div><div>Hi <b>Bob</b>,</div><div><br></div><div>Line&nbsp;one<br>two</div></div><ol start=3><li>a<ul><li>b</li></ul></li><li>c</li></ol>"),
        "Hi Bob,\n\nLine one\ntwo\n\n3. a\n  • b\n4. c"
    );
    assert_eq!(html_to_text("Thanks<blockquote><p>old</p><p>older</p></blockquote>Bye"), "Thanks\n\n> old\n>\n> older\n\nBye");
    assert_eq!(html_to_text("<pre>  a  b\n\nc</pre>"), "a  b\n\nc");
}

#[test]
fn html_to_text_tables_skipped_content_and_entities() {
    // Layout tables read top to bottom; cells in a row are spaced apart.
    assert_eq!(
        html_to_text(r#"<table><tr><td>Trouble viewing? <a href="https://t.test/view?id=12345">View online</a></td></tr><tr><td>Total</td><td>$10</td></tr></table><p>Next</p>"#),
        "Trouble viewing? View online (https://t.test/view?id=12345)\nTotal $10\n\nNext"
    );
    assert_eq!(
        html_to_text("<html><head><title>T</title><style>p{color:red}</style></head><body><script>if (a<b) document.write('</div>x')</script><p>a &amp; b &lt;c&gt;</p></body></html>"),
        "a & b <c>"
    );
    // Preview padding is invisible; an image-only link adds nothing.
    assert_eq!(html_to_text("Sale\u{200C}&zwnj;&#847;&nbsp;now <a href=\"https://x.test/\"><img src=\"a.png\"></a> <a href=\"https://x.test/\">x.test</a>"), "Sale now x.test");
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

// -- unsubscribe by email ---------------------------------------------------------

fn u_email(header: &str) -> satchel_api::UnsubscribeEmail {
    parse_unsubscribe(header, None).unwrap().email.unwrap()
}

#[test]
fn mailto_is_one_plain_address() {
    // Extra recipients smuggled through percent-encoding or separators.
    for bad in [
        "mailto:u@x.test%3E,%20%3Cboss@corp.test",
        "mailto:u@x.test,boss@corp.test",
        "mailto:u@x.test;boss@corp.test",
        "mailto:%22a%20b%22@x.test",
        "mailto:u%40x@x.test",
        "mailto:u@x.test%0D%0ABcc:boss@corp.test",
        "mailto:u@x.test%20",
        "mailto:u\\@x.test",
        "mailto:(c)u@x.test",
        "mailto:u@[127.0.0.1]",
        "mailto:@x.test",
        "mailto:u@",
        "mailto:u@x..test",
        "mailto:u%E2%80%AE@x.test",
    ] {
        assert_eq!(parse_mailto(bad), None, "{bad}");
    }
    // What's shown is what's sent, defaults included.
    let e = u_email("<mailto:unsub@x.test>");
    assert_eq!((e.subject.as_str(), e.body.as_str()), ("unsubscribe", "unsubscribe"));
    assert_eq!(u_email("<mailto:a+list@x.test?subject=a+b>").to, "a+list@x.test");
    // An invalid mailto isn't offered at all.
    assert!(parse_unsubscribe("<mailto:a@x.test%3E,%3Cb@y.test>", None).is_none());
    let u = parse_unsubscribe("<mailto:a@x.test%2Cb@y.test>, <mailto:ok@x.test>", None).unwrap();
    assert_eq!(u.mailto.as_deref(), Some("mailto:ok@x.test"));
    // Line breaks can't reach the headers; length is capped.
    let long = "a".repeat(5000);
    let (_, s, b) = parse_mailto(&format!("mailto:u@x.test?subject=Stop%0D%0ABcc:%20boss@corp.test&body={long}")).unwrap();
    assert_eq!(s, "Stop Bcc: boss@corp.test");
    assert_eq!(b.len(), crate::safety::MAILTO_BODY_MAX);
}

#[test]
fn mailto_domain_check() {
    let raw = b"From: News <news@shop.example.com>\r\nList-Unsubscribe: <mailto:u@lists.other.test>\r\nSubject: x\r\n\r\nhi\r\n";
    assert!(parse(raw).unwrap().unsubscribe.unwrap().email.unwrap().other_domain);
    let raw = b"From: News <news@shop.example.com>\r\nList-Unsubscribe: <mailto:u@bounce.example.com>\r\nSubject: x\r\n\r\nhi\r\n";
    assert!(!parse(raw).unwrap().unsubscribe.unwrap().email.unwrap().other_domain);
}

#[test]
fn bulk_headers_mark_list_and_machine_mail() {
    let bulk = |extra: &str| parse(format!("From: a@x.test\r\n{extra}Subject: x\r\n\r\nhi\r\n").as_bytes()).unwrap().bulk_headers;
    assert!(!bulk(""));
    assert!(bulk("List-Help: <https://x.test/help>\r\n"));
    assert!(bulk("List-Id: <team.x.test>\r\n"));
    assert!(bulk("Precedence: Bulk\r\n"));
    assert!(!bulk("Precedence: first-class\r\n"));
    assert!(bulk("Auto-Submitted: auto-generated\r\n"));
    assert!(!bulk("Auto-Submitted: no\r\n"));
}

#[test]
fn helpdesk_reply_marker() {
    let marked = |body: &str| parse(format!("From: a@x.test\r\nSubject: x\r\n\r\n{body}\r\n").as_bytes()).unwrap().reply_marker;
    assert!(marked("##- Please type your reply above this line -##\r\n\r\nHi David,"));
    assert!(marked("--- Please respond above this line ---"));
    assert!(marked("\u{2014}-\u{2014}-\u{2014} Reply above this line."));
    assert!(!marked("Please do not reply above this line"));
    assert!(!marked("Your statement is available. Do not reply to this email."));
    let html = "From: a@x.test\r\nContent-Type: text/html\r\nSubject: x\r\n\r\n<p><span>##- Please type your reply above this line -##</span></p><p>Hi</p>\r\n";
    assert!(parse(html.as_bytes()).unwrap().reply_marker);
}

#[test]
fn many_links_convert_in_linear_time() {
    let one = |n: usize| -> String {
        (0..n).map(|i| format!(r#"<a href="https://x.test/{i}">https://x.test/{i}</a> <a href="https://y.test/{i}">y</a> "#)).collect()
    };
    let t = html_to_text(&one(3));
    assert!(t.starts_with("https://x.test/0 y (https://y.test/0) https://x.test/1"), "{t}");
    let start = std::time::Instant::now();
    let t = html_to_text(&one(20_000));
    assert!(t.contains("https://x.test/19999 y (https://y.test/19999)"));
    assert!(!t.contains("https://x.test/5 (https://x.test/5)"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());

    let css = format!("<style>{}</style>", "a{background:URL(https://t.test/x)} ".repeat(20_000));
    let start = std::time::Instant::now();
    let s = sanitize_html(&css, false, &[]);
    assert!(s.had_remote_content && !s.html.contains("t.test"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
}

#[test]
fn huge_html_is_cut_with_a_note() {
    let big = format!("<p>{}</p>", "word ".repeat(MAX_HTML_BYTES / 4));
    let t = html_to_text(&big);
    assert!(t.len() <= MAX_HTML_BYTES + 100 && t.ends_with("[This message was too long to show in full.]"));
    let s = sanitize_html(&big, false, &[]);
    assert!(s.html.len() <= MAX_HTML_BYTES + 200 && s.html.contains("too long to show in full"));
    assert!(!sanitize_html("<p>short</p>", false, &[]).html.contains("too long"));
}

#[test]
fn css_that_loads_without_url_counts_as_remote() {
    for css in [
        "@import 'https://t.test/a.css';",
        "@IMPORT \"https://t.test/a.css\";",
        "div{background-image:image-set('https://t.test/a.png' 1x)}",
        r"div{background:\75rl(https://t.test/a.png)}",
        r"div{background:u\72l(https://t.test/a.png)}",
    ] {
        let s = sanitize_html(&format!("<style>{css}</style><p>hi</p>"), false, &[]);
        assert!(s.had_remote_content && !s.html.contains("t.test"), "{css}: {}", s.html);
        let s = sanitize_html(&format!(r#"<div style="{}">hi</div>"#, css.replace('"', "'")), false, &[]);
        assert!(s.had_remote_content && !s.html.contains("t.test"), "{css}: {}", s.html);
    }
    // Ordinary CSS is kept.
    let s = sanitize_html("<style>p{color:red}</style><p style=\"font-weight:bold\">hi</p>", false, &[]);
    assert!(!s.had_remote_content && s.html.contains("color:red") && s.html.contains("font-weight:bold"));
    // Allowed remote content keeps its CSS.
    let s = sanitize_html("<style>@import 'https://t.test/a.css';</style>", true, &[]);
    assert!(s.html.contains("t.test"));
}

#[test]
fn hostile_ids_and_types_are_dropped() {
    let raw = b"From: a@x.test\r\nMessage-ID: <ok@x>\r\nIn-Reply-To: <bad\rid@x> <good@x>\r\nReferences: <a@x> <b\rc@x> <d@x>\r\n\r\nhi\r\n";
    let p = parse(raw).unwrap();
    assert!(p.in_reply_to.as_deref().is_none_or(is_valid_message_id), "{:?}", p.in_reply_to);
    assert!(p.references.iter().all(|r| is_valid_message_id(r)), "{:?}", p.references);
    assert!(p.references.contains(&"a@x".to_string()) && p.references.contains(&"d@x".to_string()));
    assert!(!is_valid_message_id("a\r@x") && !is_valid_message_id("a b@x") && !is_valid_message_id("a>@x") && !is_valid_message_id(""));

    let o = Outgoing {
        from: Address::new("alice@x.test"),
        to: vec![Address::new("bob@x.test")],
        subject: "Re".into(),
        text: "hi".into(),
        message_id: "bad\r\nX-Evil: 1".into(),
        in_reply_to: Some("x\rBcc: c@evil.test".into()),
        references: vec!["ok@x".into(), "bad>\r\nX-Evil: 2".into()],
        attachments: vec![
            ("a.txt".into(), "text/plain\r\nX-Evil: 3".into(), b"a".to_vec()),
            ("b.ics".into(), "text/calendar; method=REPLY; charset=utf-8".into(), b"b".to_vec()),
            ("c.bin".into(), "text/plain; name=\"x\r\n\"".into(), b"c".to_vec()),
        ],
        date: chrono::Utc::now(),
        ..Default::default()
    };
    let out = String::from_utf8(build(&o)).unwrap();
    assert!(!out.contains("X-Evil") && !out.contains("Bcc:"), "{out}");
    assert!(out.contains("<ok@x>") && out.contains("text/calendar; method=REPLY; charset=utf-8"), "{out}");
    assert_eq!(out.matches("application/octet-stream").count(), 2, "{out}");
}

#[test]
fn quotes_are_stripped_however_clients_mark_them() {
    let gmail = "Thanks, see you then.\n\nOn Sat, Oct 17, 2026 at 12:23 AM Paul Christensen <\npaul@x.test> wrote:\n\n> earlier\n";
    assert_eq!(strip_quotes(gmail), "Thanks, see you then.");
    let gmail2 = "Sounds good.\nOn Fri, Sep 25, 2026 at 6:10 PM Paul <paul@x.test>\nwrote:\n> earlier\n";
    assert_eq!(strip_quotes(gmail2), "Sounds good.");
    let outlook = "Works for me.\n\n________________________________\nFrom: Bob <bob@x.test>\nSent: Monday\nTo: Alice\n\nOld text\n";
    assert_eq!(strip_quotes(outlook), "Works for me.");
    let outlook2 = "Yes.\n\nFrom: Bob <bob@x.test>\nSent: Monday, October 5, 2026 3:00 PM\nSubject: Re: x\n\nOld text\n";
    assert_eq!(strip_quotes(outlook2), "Yes.");
    // Ordinary sentences that look a little like these stay.
    let kept = "On Monday we meet.\nFrom: the team\nSee you.";
    assert_eq!(strip_quotes(kept), kept);
}
