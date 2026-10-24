//! MIME helpers: parse incoming mail into Satchel's model, sanitize HTML,
//! extract attachments, build outgoing messages, and quote/strip replies.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{MessageParser, MimeHeaders};
use satchel_api::{Address, Attachment, Unsubscribe};

mod safety;
pub use safety::{auth_failure, base_domain, deceptive_links, email_domain, is_risky_attachment, parse_mailto, parse_unsubscribe};

/// A parsed message, independent of any provider.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub subject: String,
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: Option<DateTime<Utc>>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub text: String,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
    /// Raw text/calendar parts (iMIP invitations).
    pub calendar_parts: Vec<String>,
    pub snippet: String,
    /// The topmost Authentication-Results header (added by the receiving server).
    pub auth_results: Option<String>,
    pub unsubscribe: Option<Unsubscribe>,
}

impl Parsed {
    pub fn has_attachments(&self) -> bool {
        self.attachments.iter().any(|a| !a.inline)
    }
}

fn convert_addresses(addr: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
    let Some(addr) = addr else { return vec![] };
    addr.iter()
        .filter_map(|a| {
            let email = a.address()?.trim().to_string();
            if email.is_empty() {
                return None;
            }
            Some(Address {
                name: a.name().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
                email,
            })
        })
        .collect()
}

fn header_ids(v: &mail_parser::HeaderValue<'_>) -> Vec<String> {
    if let Some(list) = v.as_text_list() {
        list.iter().map(|s| s.to_string()).collect()
    } else if let Some(t) = v.as_text() {
        vec![t.to_string()]
    } else {
        vec![]
    }
}

pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let msg = MessageParser::default().parse(raw)?;
    let text = msg
        .body_text(0)
        .map(|t| t.into_owned())
        .unwrap_or_default();
    let html = msg.body_html(0).map(|h| h.into_owned());
    // mail-parser synthesizes an HTML body from text and vice versa; only keep
    // HTML when the message actually has an HTML part, and convert it
    // ourselves when there is no text part (the synthesized text drops
    // paragraph breaks and link targets).
    let has_real_html = msg.html_bodies().any(|p| p.is_text_html());
    let has_real_text = msg.text_bodies().any(|p| p.is_text() && !p.is_text_html());
    let html = if has_real_html { html } else { None };
    let text = if !has_real_text || text.trim().is_empty() {
        html.as_deref().map(html_to_text).unwrap_or_default()
    } else {
        text
    };

    let mut attachments = Vec::new();
    let mut calendar_parts = Vec::new();
    for (i, part) in msg.attachments().enumerate() {
        let ct = part.content_type();
        let mime_type = ct
            .map(|c| match c.subtype() {
                Some(s) => format!("{}/{}", c.ctype(), s),
                None => c.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".into())
            .to_ascii_lowercase();
        if mime_type == "text/calendar" {
            if let Some(t) = part.text_contents() {
                calendar_parts.push(t.to_string());
            } else {
                calendar_parts.push(String::from_utf8_lossy(part.contents()).into_owned());
            }
        }
        let inline = part
            .content_disposition()
            .map(|d| d.is_inline())
            .unwrap_or(false)
            || (part.content_id().is_some() && part.attachment_name().is_none());
        attachments.push(Attachment {
            id: i.to_string(),
            filename: part
                .attachment_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_filename(&mime_type, i)),
            risky: !inline && is_risky_attachment(part.attachment_name().unwrap_or(""), &mime_type),
            mime_type,
            size: part.contents().len() as u64,
            inline,
            content_id: part.content_id().map(|c| c.trim_matches(['<', '>']).to_string()),
        });
    }
    // text/calendar may also appear as an alternative body part, not an attachment.
    if calendar_parts.is_empty() {
        for part in msg.parts.iter() {
            if part.is_content_type("text", "calendar")
                && let Some(t) = part.text_contents() {
                    calendar_parts.push(t.to_string());
                }
        }
    }

    let date = msg
        .date()
        .and_then(|d| Utc.timestamp_opt(d.to_timestamp(), 0).single());

    let snippet = make_snippet(&text);
    let header = |name: &str| {
        msg.headers_raw()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    let unsubscribe = header("List-Unsubscribe").and_then(|lu| parse_unsubscribe(&lu, header("List-Unsubscribe-Post").as_deref()));
    Some(Parsed {
        subject: msg.subject().unwrap_or("").trim().to_string(),
        from: convert_addresses(msg.from()).into_iter().next(),
        to: convert_addresses(msg.to()),
        cc: convert_addresses(msg.cc()),
        bcc: convert_addresses(msg.bcc()),
        reply_to: convert_addresses(msg.reply_to()),
        date,
        message_id: msg.message_id().map(|s| s.to_string()),
        in_reply_to: header_ids(msg.in_reply_to()).into_iter().next(),
        references: header_ids(msg.references()),
        text,
        html,
        attachments,
        calendar_parts,
        snippet,
        auth_results: header("Authentication-Results"),
        unsubscribe,
    })
}

fn default_filename(mime: &str, i: usize) -> String {
    let ext = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "text/calendar" => "ics",
        "text/plain" => "txt",
        _ => "bin",
    };
    format!("attachment-{}.{ext}", i + 1)
}

/// Returns (filename, mime_type, bytes) for an attachment by id.
pub fn attachment_bytes(raw: &[u8], id: &str) -> Option<(String, String, Vec<u8>)> {
    let parsed = parse(raw)?;
    let meta = parsed.attachments.iter().find(|a| a.id == id)?.clone();
    let msg = MessageParser::default().parse(raw)?;
    let idx: usize = id.parse().ok()?;
    let part = msg.attachments().nth(idx)?;
    Some((meta.filename, meta.mime_type, part.contents().to_vec()))
}

/// Attachments that are referenced inline by content id, as data URLs.
pub fn inline_data_urls(raw: &[u8]) -> Vec<(String, String)> {
    use base64::Engine;
    let Some(msg) = MessageParser::default().parse(raw) else { return vec![] };
    let mut out = vec![];
    for part in msg.attachments() {
        let Some(cid) = part.content_id() else { continue };
        let ct = part
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        if !ct.starts_with("image/") {
            continue;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(part.contents());
        out.push((cid.trim_matches(['<', '>']).to_string(), format!("data:{ct};base64,{b64}")));
    }
    out
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

pub struct Sanitized {
    pub html: String,
    /// The original referenced remote resources that were blocked.
    pub had_remote_content: bool,
}

/// Sanitize untrusted HTML. Scripts, event handlers, forms and frames are
/// removed. Remote images are dropped unless `allow_remote`; `cid:` images are
/// replaced from `inline` (cid -> data URL). The UI additionally renders the
/// result in a sandboxed iframe with a CSP, which also covers CSS `url()`.
pub fn sanitize_html(html: &str, allow_remote: bool, inline: &[(String, String)]) -> Sanitized {
    let had_remote = Arc::new(AtomicBool::new(false));
    let flag = had_remote.clone();
    let inline: Vec<(String, String)> = inline.to_vec();

    let mut b = ammonia::Builder::default();
    b.add_tags(["style", "font", "center", "big", "small", "u", "s"]);
    b.rm_clean_content_tags(["style"]);
    b.add_generic_attributes(["style", "class", "align", "valign", "width", "height", "bgcolor", "color", "dir", "role"]);
    b.add_tag_attributes("table", ["cellpadding", "cellspacing", "border"]);
    b.add_tag_attributes("td", ["colspan", "rowspan", "nowrap"]);
    b.add_tag_attributes("th", ["colspan", "rowspan"]);
    b.add_tag_attributes("font", ["face", "size"]);
    b.add_tag_attributes("img", ["src", "alt", "title", "width", "height"]);
    // Markers mail clients put on quoted history, so the reader can fold it.
    b.add_tag_attributes("div", ["id"]);
    b.add_tag_attributes("blockquote", ["type"]);
    b.url_schemes(HashSet::from(["http", "https", "mailto", "data", "cid", "tel"]));
    b.link_rel(Some("noopener noreferrer nofollow"));
    b.attribute_filter(move |element, attribute, value| {
        let lower = value.trim().to_ascii_lowercase();
        if element == "img" && attribute == "src" {
            if let Some(cid) = lower.strip_prefix("cid:") {
                let found = inline
                    .iter()
                    .find(|(c, _)| c.eq_ignore_ascii_case(cid))
                    .map(|(_, url)| url.clone());
                return found.map(Cow::Owned);
            }
            if lower.starts_with("data:image/") {
                return Some(Cow::Borrowed(value));
            }
            if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//") {
                flag.store(true, Ordering::Relaxed);
                return if allow_remote { Some(Cow::Borrowed(value)) } else { None };
            }
            return None;
        }
        if attribute == "style" && lower.contains("url(") {
            flag.store(true, Ordering::Relaxed);
            if !allow_remote {
                return Some(Cow::Owned(strip_css_urls(value)));
            }
        }
        if element == "a" && attribute == "href" && lower.starts_with("data:") {
            return None;
        }
        Some(Cow::Borrowed(value))
    });
    let mut out = b.clean(html).to_string();
    if !allow_remote && out.contains("url(") {
        // <style> blocks may also pull remote resources.
        had_remote.store(true, Ordering::Relaxed);
        out = strip_css_urls(&out);
    }
    Sanitized { html: out, had_remote_content: had_remote.load(Ordering::Relaxed) }
}

fn strip_css_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.to_ascii_lowercase().find("url(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 4..];
        match after.find(')') {
            Some(j) => {
                out.push_str("none");
                rest = &after[j + 1..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Readable text for HTML-only mail: paragraphs kept, links written inline as
/// `text (url)`, no markdown markers and no hard wrapping (the reader reflows).
pub fn html_to_text(html: &str) -> String {
    let urls = Rc::new(RefCell::new(Vec::new()));
    let text = html2text::config::with_decorator(MailTextDecorator { links: vec![], urls: urls.clone() })
        .no_table_borders()
        .no_link_wrapping()
        .allow_width_overflow()
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_default();
    let mut text = text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    // A link whose text is its own address doesn't need it twice.
    for url in urls.borrow().iter() {
        text = text.replace(&format!("{url} ({url})"), url);
    }
    text.trim().to_string()
}

#[derive(Clone)]
struct MailTextDecorator {
    /// What to write at the end of each open link.
    links: Vec<String>,
    urls: Rc<RefCell<Vec<String>>>,
}

impl html2text::render::TextDecorator for MailTextDecorator {
    type Annotation = ();

    fn decorate_link_start(&mut self, url: &str) -> (String, ()) {
        let target = url.strip_prefix("mailto:").unwrap_or(url);
        let shown = url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:");
        self.links.push(if shown { format!(" ({target})") } else { String::new() });
        if shown {
            self.urls.borrow_mut().push(target.to_string());
        }
        (String::new(), ())
    }
    fn decorate_link_end(&mut self) -> String {
        self.links.pop().unwrap_or_default()
    }
    fn decorate_em_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_em_end(&self) -> String {
        String::new()
    }
    fn decorate_strong_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strong_end(&self) -> String {
        String::new()
    }
    fn decorate_strikeout_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strikeout_end(&self) -> String {
        String::new()
    }
    fn decorate_code_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_code_end(&self) -> String {
        String::new()
    }
    fn decorate_preformat_first(&self) {}
    fn decorate_preformat_cont(&self) {}
    // Images are left out so an image-only message still has no text.
    fn decorate_image(&mut self, _src: &str, _title: &str) -> (String, ()) {
        (String::new(), ())
    }
    fn header_prefix(&self, _level: usize) -> String {
        String::new()
    }
    fn quote_prefix(&self) -> String {
        "> ".into()
    }
    fn unordered_item_prefix(&self) -> String {
        "• ".into()
    }
    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{i}. ")
    }
    fn make_subblock_decorator(&self) -> Self {
        Self { links: vec![], urls: self.urls.clone() }
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn is_attribution_line(line: &str) -> bool {
    let l = line.trim();
    (l.starts_with("On ") && l.ends_with("wrote:"))
        || (l.starts_with("Le ") && l.ends_with("a écrit :"))
        || (l.starts_with("Am ") && l.ends_with("schrieb:"))
        || l == "-----Original Message-----"
        || l.starts_with("---------- Forwarded message")
}

/// Remove quoted history ("> ..." lines and "On X wrote:" attributions) and
/// trailing signatures, leaving what this message actually added.
pub fn strip_quotes(text: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for line in text.lines() {
        if is_attribution_line(line) {
            break;
        }
        if line.trim_start().starts_with('>') {
            continue;
        }
        if line == "-- " {
            break;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

pub fn make_snippet(text: &str) -> String {
    let stripped = strip_quotes(text);
    let source = if stripped.trim().is_empty() { text } else { &stripped };
    let collapsed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, 200)
}

pub fn truncate_chars(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Strip leading Re:/Fwd:/AW:/etc for thread grouping.
pub fn normalize_subject(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let mut changed = false;
        for p in ["re:", "fwd:", "fw:", "aw:", "wg:", "sv:", "vs:", "re :", "fwd :"] {
            if lower.starts_with(p) {
                s = s[p.len()..].trim_start();
                changed = true;
                break;
            }
        }
        if lower.starts_with("re[")
            && let Some(i) = s.find("]:") {
                s = s[i + 2..].trim_start();
                changed = true;
            }
        if !changed {
            break;
        }
    }
    s.to_string()
}

pub fn reply_subject(s: &str) -> String {
    let n = normalize_subject(s);
    format!("Re: {n}")
}

pub fn forward_subject(s: &str) -> String {
    let n = normalize_subject(s);
    format!("Fwd: {n}")
}

pub fn quote_for_reply(text: &str, from: Option<&Address>, date: Option<DateTime<Utc>>) -> String {
    let who = from.map(|a| a.display()).unwrap_or_else(|| "someone".into());
    let when = date
        .map(|d| d.format("%a, %b %-d, %Y at %-I:%M %p UTC").to_string())
        .unwrap_or_default();
    let mut out = format!("\n\nOn {when}, {who} wrote:\n");
    for line in text.lines() {
        if line.starts_with('>') {
            out.push('>');
        } else {
            out.push_str("> ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn forward_body(p: &Parsed) -> String {
    let mut out = String::from("\n\n---------- Forwarded message ---------\n");
    if let Some(f) = &p.from {
        out.push_str(&format!("From: {}\n", f.display()));
    }
    if let Some(d) = p.date {
        out.push_str(&format!("Date: {}\n", d.to_rfc2822()));
    }
    out.push_str(&format!("Subject: {}\n", p.subject));
    if !p.to.is_empty() {
        out.push_str(&format!(
            "To: {}\n",
            p.to.iter().map(|a| a.display()).collect::<Vec<_>>().join(", ")
        ));
    }
    out.push('\n');
    out.push_str(&p.text);
    out
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Outgoing {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// (filename, mime type, bytes)
    pub attachments: Vec<(String, String, Vec<u8>)>,
    /// Without angle brackets.
    pub message_id: String,
    pub date: DateTime<Utc>,
    /// Extra headers, e.g. X-Satchel-Draft-Id.
    pub extra_headers: Vec<(String, String)>,
    /// Include Bcc header (true for drafts saved to the server, false for sent mail).
    pub include_bcc_header: bool,
}

fn builder_addr(a: &Address) -> mail_builder::headers::address::Address<'static> {
    mail_builder::headers::address::Address::new_address(a.name.clone(), a.email.clone())
}

fn builder_list(v: &[Address]) -> mail_builder::headers::address::Address<'static> {
    mail_builder::headers::address::Address::new_list(v.iter().map(builder_addr).collect())
}

pub fn new_message_id(domain: &str) -> String {
    format!("{}@{}", uuid_like(), domain)
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let r: u64 = (n as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    format!("{:x}.{:x}", n, r)
}

pub fn build(o: &Outgoing) -> Vec<u8> {
    let mut b = mail_builder::MessageBuilder::new()
        .from(builder_addr(&o.from))
        .subject(o.subject.clone())
        .message_id(o.message_id.clone())
        .date(o.date.timestamp());
    if !o.to.is_empty() {
        b = b.to(builder_list(&o.to));
    }
    if !o.cc.is_empty() {
        b = b.cc(builder_list(&o.cc));
    }
    if o.include_bcc_header && !o.bcc.is_empty() {
        b = b.bcc(builder_list(&o.bcc));
    }
    if let Some(irt) = &o.in_reply_to {
        b = b.in_reply_to(irt.clone());
    }
    if !o.references.is_empty() {
        b = b.references(o.references.clone());
    }
    for (k, v) in &o.extra_headers {
        b = b.header(k.clone(), mail_builder::headers::raw::Raw::new(v.clone()));
    }
    b = b.text_body(o.text.clone());
    if let Some(h) = &o.html {
        b = b.html_body(h.clone());
    }
    for (name, mime, data) in &o.attachments {
        b = b.attachment(mime.clone(), name.clone(), data.clone());
    }
    b.write_to_vec().unwrap_or_default()
}

/// Escape text for inclusion in HTML.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plain text to minimal HTML with links and quote styling.
pub fn text_to_html(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let esc = escape_html(line);
        if line.starts_with('>') {
            out.push_str(&format!("<div class=\"q\">{esc}</div>"));
        } else {
            out.push_str(&format!("<div>{}</div>", if esc.is_empty() { "<br>".into() } else { esc }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
}

