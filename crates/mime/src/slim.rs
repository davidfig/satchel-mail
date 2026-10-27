//! Copies of messages without their attachments, for storing mail locally
//! without the disk cost of every file anyone ever sent.

use super::*;

/// Inline images up to this size stay (logos, signatures), so most messages
/// still render without fetching the full copy.
pub const KEEP_INLINE_BYTES: usize = 64 * 1024;

/// Messages smaller than this download whole: there is too little to leave
/// out to be worth asking the server for the structure first.
pub const SLIM_FROM_BYTES: u64 = 128 * 1024;

/// `raw` with the contents of its attachments removed: each attachment part
/// keeps its headers (name, type, Content-ID) and loses its body, so the
/// attachment list and ids stay the same. Calendar invitations and small
/// inline images are kept. Returns `None` when there is nothing to remove,
/// or when the result would not parse to the same text and attachments.
pub fn strip_attachments(raw: &[u8]) -> Option<Vec<u8>> {
    let msg = MessageParser::default().parse(raw)?;
    let mut cut: Vec<(usize, usize)> = msg
        .attachments()
        .filter(|p| !p.is_content_type("text", "calendar"))
        .filter(|p| !(p.content_id().is_some() && p.contents().len() <= KEEP_INLINE_BYTES && !p.content_disposition().is_some_and(|d| d.is_attachment())))
        .map(|p| (p.offset_body as usize, p.offset_end as usize))
        .filter(|(start, end)| start < end && *end <= raw.len())
        .collect();
    if cut.is_empty() {
        return None;
    }
    cut.sort_unstable();
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    for (start, end) in cut {
        if start < at {
            continue; // nested inside a part already removed
        }
        out.extend_from_slice(&raw[at..start]);
        at = end;
    }
    out.extend_from_slice(&raw[at..]);
    // Only keep the copy if it reads the same apart from attachment contents.
    let (before, after) = (parse(raw)?, parse(&out)?);
    let same_atts = before.attachments.len() == after.attachments.len()
        && before.attachments.iter().zip(&after.attachments).all(|(a, b)| a.id == b.id && a.filename == b.filename && a.mime_type == b.mime_type && a.inline == b.inline);
    (same_atts && before.text == after.text && before.html == after.html && before.calendar_parts == after.calendar_parts).then_some(out)
}
