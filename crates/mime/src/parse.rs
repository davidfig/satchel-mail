//! Parsing incoming mail into Satchel's model, and pulling out attachments.

use super::*;

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

pub(crate) fn convert_addresses(addr: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
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

pub(crate) fn header_ids(v: &mail_parser::HeaderValue<'_>) -> Vec<String> {
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

pub(crate) fn default_filename(mime: &str, i: usize) -> String {
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
