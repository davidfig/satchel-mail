//! Building outgoing messages.

use super::*;

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

pub(crate) fn builder_addr(a: &Address) -> mail_builder::headers::address::Address<'static> {
    mail_builder::headers::address::Address::new_address(a.name.clone(), a.email.clone())
}

pub(crate) fn builder_list(v: &[Address]) -> mail_builder::headers::address::Address<'static> {
    mail_builder::headers::address::Address::new_list(v.iter().map(builder_addr).collect())
}

pub fn new_message_id(domain: &str) -> String {
    format!("{}@{}", uuid_like(), domain)
}

pub(crate) fn uuid_like() -> String {
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
