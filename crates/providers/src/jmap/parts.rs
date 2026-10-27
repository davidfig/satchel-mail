//! Downloading a message without its attachments: `bodyStructure` describes
//! every part, `bodyValues` holds the text, and other kept parts download
//! as blobs (0036).

use super::*;

use satchel_mime::{Body, Part};

const BODY_PROPS: [&str; 7] = ["partId", "blobId", "size", "headers", "type", "disposition", "cid"];

fn is_multipart(p: &Value) -> bool {
    p["type"].as_str().is_some_and(|t| t.to_ascii_lowercase().starts_with("multipart/"))
}

/// The sub-parts of `p` and whether they sit in a multipart/alternative.
fn children(p: &Value) -> (Vec<Value>, bool) {
    let alt = p["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("multipart/alternative"));
    (p["subParts"].as_array().cloned().unwrap_or_default(), alt)
}

fn keeps(p: &Value, in_alt: bool) -> bool {
    satchel_mime::keeps_contents(p["type"].as_str().unwrap_or("text/plain"), p["disposition"].as_str(), p["cid"].is_string(), p["size"].as_u64().unwrap_or(0), in_alt)
}

/// Whether any part's contents can stay on the server.
pub(super) fn skips_any(p: &Value, in_alt: bool) -> bool {
    if !is_multipart(p) {
        return !keeps(p, in_alt);
    }
    let (kids, alt) = children(p);
    kids.iter().any(|c| skips_any(c, alt))
}

/// A kept part's text, if the server sent all of it.
fn text_value<'a>(p: &Value, values: &'a Value) -> Option<&'a str> {
    let v = &values[p["partId"].as_str()?];
    (v["isTruncated"] != true && v["isEncodingProblem"] != true).then(|| v["value"].as_str()).flatten()
}

/// Blob ids of kept parts without a text value.
pub(super) fn to_fetch(p: &Value, in_alt: bool, values: &Value, out: &mut Vec<String>) {
    if is_multipart(p) {
        let (kids, alt) = children(p);
        kids.iter().for_each(|c| to_fetch(c, alt, values, out));
    } else if keeps(p, in_alt) && text_value(p, values).is_none()
        && let Some(b) = p["blobId"].as_str() {
            out.push(b.to_string());
        }
}

/// The part tree, with downloaded blobs by id.
pub(super) fn part(p: &Value, in_alt: bool, values: &Value, blobs: &HashMap<String, Vec<u8>>) -> Option<Part> {
    let fields = p["headers"].as_array()?.iter().filter_map(|h| Some((h["name"].as_str()?, h["value"].as_str()?.trim_start_matches([' ', '\t']))));
    let headers = satchel_mime::header_block(fields);
    let body = if is_multipart(p) {
        let (kids, alt) = children(p);
        Body::Multipart(kids.iter().map(|c| part(c, alt, values, blobs)).collect::<Option<_>>()?)
    } else if keeps(p, in_alt) {
        match text_value(p, values) {
            Some(t) => Body::Text(t.to_string()),
            None => Body::Decoded(blobs.get(p["blobId"].as_str()?)?.clone()),
        }
    } else {
        Body::Skipped(p["size"].as_u64().unwrap_or(0))
    };
    Some(Part { headers, body })
}

impl JmapProvider {
    /// The message without attachment contents, or `None` if it has none to
    /// leave out or doesn't rebuild (the caller then downloads it whole).
    pub(super) async fn fetch_slim(&self, id: &str) -> Result<Option<satchel_mime::Slim>> {
        let s = self.session().await?;
        let args = json!({ "accountId": s.account_id, "ids": [id], "properties": ["bodyStructure", "bodyValues"], "bodyProperties": BODY_PROPS,
            "fetchTextBodyValues": true, "fetchHTMLBodyValues": true });
        let r = self.call(vec![json!(["Email/get", args, "b"])]).await?;
        let e = &r["b"]["list"][0];
        let (root, values) = (&e["bodyStructure"], &e["bodyValues"]);
        if root.is_null() || !skips_any(root, false) {
            return Ok(None);
        }
        let mut ids = vec![];
        to_fetch(root, false, values, &mut ids);
        let mut blobs = HashMap::new();
        for b in ids {
            let data = self.download(&s, &b, "application/octet-stream").await?;
            blobs.insert(b, data);
        }
        Ok(part(root, false, values, &blobs).and_then(|p| satchel_mime::rebuild(&p)))
    }
}
