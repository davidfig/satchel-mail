//! Downloading a message without its attachments: `format=full` describes
//! every part, with the contents of small ones inline and an attachment id
//! for the rest (0036).

use super::*;

use satchel_mime::{Body, Part};

fn header<'a>(p: &'a Value, name: &str) -> Option<&'a str> {
    p["headers"].as_array()?.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))?["value"].as_str()
}

fn is_multipart(p: &Value) -> bool {
    p["mimeType"].as_str().is_some_and(|t| t.to_ascii_lowercase().starts_with("multipart/"))
}

/// The sub-parts of `p` and whether they sit in a multipart/alternative.
fn children(p: &Value) -> (Vec<Value>, bool) {
    let alt = p["mimeType"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("multipart/alternative"));
    (p["parts"].as_array().cloned().unwrap_or_default(), alt)
}

fn keeps(p: &Value, in_alt: bool) -> bool {
    let disposition = header(p, "Content-Disposition").map(|d| d.split(';').next().unwrap_or_default().trim());
    satchel_mime::keeps_contents(p["mimeType"].as_str().unwrap_or("text/plain"), disposition, header(p, "Content-ID").is_some(), p["body"]["size"].as_u64().unwrap_or(0), in_alt)
}

/// Whether any part's contents can stay on the server.
pub(super) fn skips_any(p: &Value, in_alt: bool) -> bool {
    if !is_multipart(p) {
        return !keeps(p, in_alt);
    }
    let (kids, alt) = children(p);
    kids.iter().any(|c| skips_any(c, alt))
}

/// Attachment ids of kept parts whose contents didn't come inline.
pub(super) fn to_fetch(p: &Value, in_alt: bool, out: &mut Vec<String>) {
    if is_multipart(p) {
        let (kids, alt) = children(p);
        kids.iter().for_each(|c| to_fetch(c, alt, out));
    } else if keeps(p, in_alt) && p["body"]["data"].is_null()
        && let Some(id) = p["body"]["attachmentId"].as_str() {
            out.push(id.to_string());
        }
}

/// The part tree, with fetched contents by attachment id. Gmail's data is
/// the contents with the transfer encoding undone, in the part's charset.
pub(super) fn part(p: &Value, in_alt: bool, fetched: &HashMap<String, Vec<u8>>) -> Option<Part> {
    let headers = satchel_mime::header_block(p["headers"].as_array()?.iter().filter_map(|h| Some((h["name"].as_str()?, h["value"].as_str()?))));
    let body = if is_multipart(p) {
        let (kids, alt) = children(p);
        Body::Multipart(kids.iter().map(|c| part(c, alt, fetched)).collect::<Option<_>>()?)
    } else if keeps(p, in_alt) {
        match (p["body"]["data"].as_str(), p["body"]["attachmentId"].as_str()) {
            (Some(d), _) => Body::Decoded(b64url_decode(d).ok()?),
            (None, Some(id)) => Body::Decoded(fetched.get(id)?.clone()),
            (None, None) => Body::Decoded(vec![]),
        }
    } else {
        Body::Skipped(p["body"]["size"].as_u64().unwrap_or(0))
    };
    Some(Part { headers, body })
}

impl GmailProvider {
    /// The message without attachment contents, or `None` if it has none to
    /// leave out or doesn't rebuild (the caller then downloads it whole).
    pub(super) async fn fetch_slim(&self, id: &str) -> Result<Option<satchel_mime::Slim>> {
        let m = self.get(&format!("{GMAIL}/messages/{}?format=full", crate::net::enc(id)), "get message").await?;
        let payload = &m["payload"];
        if !skips_any(payload, false) {
            return Ok(None);
        }
        let mut ids = vec![];
        to_fetch(payload, false, &mut ids);
        let mut fetched = HashMap::new();
        for a in ids {
            let v = self.get(&format!("{GMAIL}/messages/{}/attachments/{}", crate::net::enc(id), crate::net::enc(&a)), "get attachment").await?;
            fetched.insert(a, b64url_decode(v["data"].as_str().context("no attachment data")?)?);
        }
        Ok(part(payload, false, &fetched).and_then(|p| satchel_mime::rebuild(&p)))
    }
}
