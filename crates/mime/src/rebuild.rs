// SPDX-License-Identifier: MIT OR Apache-2.0
//! Rebuilding a message from its parts as a server describes them, with
//! attachment contents left out so they are never downloaded. The result is
//! what [`strip_attachments`] makes of the full message.

use super::*;

use std::collections::HashMap;

/// One MIME part as a server describes it.
pub struct Part {
    /// Its header fields, each ending in CRLF, without the blank line.
    pub headers: Vec<u8>,
    pub body: Body,
}

pub enum Body {
    /// Sub-parts; the boundary comes from the Content-Type header.
    Multipart(Vec<Part>),
    /// Contents as sent, in the part's Content-Transfer-Encoding.
    Encoded(Vec<u8>),
    /// Contents with the transfer encoding undone, still in the part's charset.
    Decoded(Vec<u8>),
    /// Text already decoded from its charset.
    Text(String),
    /// Not downloaded: the decoded size.
    Skipped(u64),
}

/// A message without attachment contents, with the attachment list (and
/// real sizes) of the full message.
#[derive(Debug, Clone)]
pub struct Slim {
    pub raw: Vec<u8>,
    pub attachments: Vec<Attachment>,
}

/// Whether a stored copy keeps a part's contents, judged from what a server
/// says about it before downloading anything. Calendar parts, text that isn't
/// an attachment, and small inline images are kept. `in_alternative`: the
/// parent is multipart/alternative, where text is always a body.
pub fn keeps_contents(mime_type: &str, disposition: Option<&str>, has_content_id: bool, size: u64, in_alternative: bool) -> bool {
    let mime_type = mime_type.to_ascii_lowercase();
    let attachment = disposition.is_some_and(|d| d.eq_ignore_ascii_case("attachment"));
    if mime_type == "text/calendar" {
        return true;
    }
    if mime_type.starts_with("multipart/") {
        return true;
    }
    if mime_type.starts_with("text/") {
        return !attachment || in_alternative;
    }
    !mime_type.starts_with("message/") && has_content_id && !attachment && size <= KEEP_INLINE_BYTES as u64
}

/// Header fields from name/value pairs, for servers that list them that way.
pub fn header_block<'a>(fields: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, v) in fields {
        out.extend_from_slice(format!("{n}: {v}\r\n").as_bytes());
    }
    out
}

/// The message `root` describes, with the contents of attachment parts
/// removed as [`strip_attachments`] would. `None` when it can't be rebuilt
/// faithfully (a multipart without a boundary, or a structure that doesn't
/// parse back the same); callers then download the full message.
pub fn rebuild(root: &Part) -> Option<Slim> {
    let mut out = Vec::new();
    let mut skipped = HashMap::new();
    let mut count = 0;
    write_part(root, &mut out, &mut skipped, &mut count)?;
    let msg = MessageParser::default().parse(&out)?;
    if msg.parts.len() != count {
        return None;
    }
    let mut parsed = parse(&out)?;
    if parsed.attachments.len() != msg.attachments.len() {
        return None;
    }
    for (a, part) in parsed.attachments.iter_mut().zip(&msg.attachments) {
        if let Some(size) = skipped.get(&(*part as usize)) {
            a.size = *size;
        }
    }
    // Parts fetched only because they couldn't be told apart beforehand
    // (text attachments, say) go the same way as in a stripped copy.
    let raw = strip_attachments(&out).unwrap_or(out);
    Some(Slim { raw, attachments: parsed.attachments })
}

/// Write `p` in place, numbering parts in the order the parser lists them.
fn write_part(p: &Part, out: &mut Vec<u8>, skipped: &mut HashMap<usize, u64>, count: &mut usize) -> Option<()> {
    let index = *count;
    *count += 1;
    match &p.body {
        Body::Multipart(parts) => {
            let (_, attrs) = content_type_of(&p.headers)?;
            let boundary = attrs.into_iter().find(|(k, _)| k.eq_ignore_ascii_case("boundary"))?.1;
            out.extend_from_slice(&p.headers);
            out.extend_from_slice(b"\r\n");
            for sub in parts {
                out.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
                write_part(sub, out, skipped, count)?;
            }
            out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        }
        Body::Encoded(b) => {
            out.extend_from_slice(&p.headers);
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(b);
        }
        Body::Decoded(b) => write_base64(&rewrite_headers(&p.headers, false), b, out),
        Body::Text(t) => write_base64(&rewrite_headers(&p.headers, true), t.as_bytes(), out),
        Body::Skipped(size) => {
            skipped.insert(index, *size);
            out.extend_from_slice(&p.headers);
            out.extend_from_slice(b"\r\n");
        }
    }
    Some(())
}

fn write_base64(headers: &[u8], data: &[u8], out: &mut Vec<u8>) {
    use base64::Engine;
    out.extend_from_slice(headers);
    out.extend_from_slice(b"\r\n");
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    for (i, line) in b64.as_bytes().chunks(76).enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(line);
    }
}

/// A header block's Content-Type as (type/subtype, parameters).
fn content_type_of(headers: &[u8]) -> Option<(String, Vec<(String, String)>)> {
    let mut block = headers.to_vec();
    block.extend_from_slice(b"\r\n");
    let msg = MessageParser::default().parse(&block)?;
    let c = msg.content_type()?;
    let attrs = c.attributes().unwrap_or_default().iter().map(|a| (a.name.to_string(), a.value.to_string())).collect();
    Some((format!("{}/{}", c.ctype(), c.subtype().unwrap_or("plain")), attrs))
}

/// Header fields one per entry, continuation lines included.
fn fields(headers: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = vec![];
    let mut start = 0;
    for (i, _) in headers.iter().enumerate().filter(|(_, b)| **b == b'\n') {
        let next = headers.get(i + 1);
        if !matches!(next, Some(b' ' | b'\t')) {
            out.push(&headers[start..=i]);
            start = i + 1;
        }
    }
    if start < headers.len() {
        out.push(&headers[start..]);
    }
    out
}

fn is_field(f: &[u8], name: &str) -> bool {
    f.iter().position(|b| *b == b':').is_some_and(|c| f[..c].trim_ascii().eq_ignore_ascii_case(name.as_bytes()))
}

/// The headers for contents written out as base64; with `utf8`, the
/// Content-Type's charset becomes UTF-8 as well.
fn rewrite_headers(headers: &[u8], utf8: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(headers.len() + 40);
    for f in fields(headers) {
        if is_field(f, "Content-Transfer-Encoding") {
            continue;
        }
        if utf8 && is_field(f, "Content-Type") {
            continue;
        }
        out.extend_from_slice(f);
        if !f.ends_with(b"\n") {
            out.extend_from_slice(b"\r\n");
        }
    }
    if utf8 {
        let (mut v, attrs) = content_type_of(headers).unwrap_or_else(|| ("text/plain".into(), vec![]));
        for (k, val) in attrs {
            if !k.eq_ignore_ascii_case("charset") {
                v.push_str(&format!("; {k}=\"{}\"", val.replace('\\', "\\\\").replace('"', "\\\"")));
            }
        }
        out.extend_from_slice(format!("Content-Type: {v}; charset=utf-8\r\n").as_bytes());
    }
    out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n");
    out
}
