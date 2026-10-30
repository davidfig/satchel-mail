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

/// Whether `id` (without angle brackets) is safe to write into Message-ID,
/// In-Reply-To or References: no whitespace, control characters or angle
/// brackets, which could end the header or the id early.
pub fn is_valid_message_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 998 && !id.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>')
}

/// A MIME type safe to write as a part's Content-Type: `type/subtype` made
/// of token characters, optionally with simple `; name=value` parameters.
/// Anything else (line breaks, quotes, stray separators) becomes
/// application/octet-stream, so it can't add part headers.
pub(crate) fn safe_mime_type(m: &str) -> String {
    let token = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c));
    let mut parts = m.split(';');
    let ok = parts.next().and_then(|t| t.trim().split_once('/')).is_some_and(|(a, b)| token(a) && token(b))
        && parts.all(|p| p.trim().split_once('=').is_some_and(|(k, v)| token(k.trim()) && token(v.trim())));
    if ok { m.trim().to_string() } else { "application/octet-stream".into() }
}

pub fn build(o: &Outgoing) -> Vec<u8> {
    let message_id = if is_valid_message_id(&o.message_id) {
        o.message_id.clone()
    } else {
        let domain = email_domain(&o.from.email);
        new_message_id(if is_valid_message_id(&domain) { &domain } else { "satchel.local" })
    };
    let mut b = mail_builder::MessageBuilder::new()
        .from(builder_addr(&o.from))
        .subject(o.subject.clone())
        .message_id(message_id)
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
    // Ids can come from hostile mail; drop any that could break the header.
    if let Some(irt) = o.in_reply_to.as_ref().filter(|i| is_valid_message_id(i)) {
        b = b.in_reply_to(irt.clone());
    }
    let references: Vec<String> = o.references.iter().filter(|r| is_valid_message_id(r)).cloned().collect();
    if !references.is_empty() {
        b = b.references(references);
    }
    for (k, v) in &o.extra_headers {
        b = b.header(k.clone(), mail_builder::headers::raw::Raw::new(v.clone()));
    }
    b = b.text_body(o.text.clone());
    if let Some(h) = &o.html {
        b = b.html_body(h.clone());
    }
    for (name, mime, data) in &o.attachments {
        b = b.attachment(safe_mime_type(mime), name.clone(), data.clone());
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
