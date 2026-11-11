//! Attachment files in thread folders (0078). For messages inside the
//! attachment window they're written from the stored copy when it has them,
//! or downloaded in the background for the folder alone (see
//! `engine::ai_attachments`). Satchel never reads them back: they're the
//! only copy, and deleting one loses nothing the server doesn't have.

use super::*;

use chrono::{DateTime, Duration};

/// Larger attachments are left out.
pub(crate) const MAX_ATTACHMENT: u64 = 25 * 1024 * 1024;

/// Whether a part is something the sender attached, as the folder and
/// search's `filename:` and `has:<kind>` count it (0077): not an image the
/// HTML shows in place (a logo in a signature), nor Gmail's AMP body, which
/// comes named like an attachment but is another version of the message.
/// Apple Mail marks attached documents inline, so inline alone doesn't rule
/// one out.
pub(crate) fn listed_attachment(a: &Attachment) -> bool {
    let shown_in_place = a.inline && a.content_id.is_some() && a.mime_type.to_ascii_lowercase().starts_with("image/");
    !shown_in_place && !a.mime_type.eq_ignore_ascii_case("text/x-amp-html")
}

/// The file each of the `n`th message's attachments is written to, in the
/// thread's `attachments/`: `<n>-<name>`. Only those the folder takes.
pub(super) fn attachment_files(n: usize, atts: &[&Attachment]) -> Vec<(String, String)> {
    let mut names = Names::reserving(&[]);
    atts.iter()
        .filter(|a| a.size <= MAX_ATTACHMENT)
        .map(|a| {
            let safe = safe_name(&a.filename);
            let (stem, ext) = match safe.rsplit_once('.') {
                Some((s, e)) if !s.is_empty() && e.len() <= 10 => (s.to_string(), format!(".{e}")),
                _ => (safe.clone(), String::new()),
            };
            (a.id.clone(), names.take(&format!("{n}-{stem}"), &ext, &format!("{}\0{}", a.id, a.filename)))
        })
        .collect()
}

impl Mirror<'_> {
    /// Messages dated from when on have their attachments written: None if
    /// none do, `Some(None)` for every message the folder shows.
    pub(crate) fn attachment_window(&self) -> Option<Option<DateTime<Utc>>> {
        if !self.settings.include_attachments || !RAW_FILES_EXPORTED {
            return None;
        }
        let own = self.settings.attachment_days.map(|d| Utc::now() - Duration::days(d as i64));
        Some(match (own, self.cutoff()) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        })
    }

    /// Write one message's attachments from its full raw message, downloaded
    /// for the folder. Returns false if its thread isn't written, no longer
    /// shows the message, or the message lacks one of them.
    pub fn write_attachments(&self, thread_id: &str, message_id: &str, raw: &[u8]) -> Result<bool> {
        let Some(rel) = self.store.mirror_path(thread_id)? else { return Ok(false) };
        let msgs = self.visible_messages(thread_id)?;
        let Some((i, m)) = msgs.iter().enumerate().find(|(_, m)| m.summary.id == message_id) else { return Ok(false) };
        let dir = self.root().join(rel).join("attachments");
        let atts: Vec<&Attachment> = m.attachments.iter().filter(|a| listed_attachment(a)).collect();
        let mut all = true;
        for (id, file) in attachment_files(i + 1, &atts) {
            match satchel_mime::attachment_bytes(raw, &id) {
                Some((_, _, data)) => write_if_changed(&self.root(), &dir.join(file), &data)?,
                None => all = false,
            }
        }
        Ok(all)
    }
}
