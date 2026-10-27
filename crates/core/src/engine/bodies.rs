//! Downloading message content: in the background, newest first and within
//! the storage limit, or when a message or attachment is asked for (0036).

use super::*;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{debug, warn};

use crate::provider::{Download, MailProvider, SyncCtx};
use crate::store::StoredBlob;

/// What a message with no known size is assumed to take, and the most any
/// one is assumed to take (attachments are left out, so big server sizes
/// overstate it).
const UNKNOWN_SIZE: u64 = 256 * 1024;
const MAX_ESTIMATE: u64 = 1024 * 1024;

impl Engine {
    /// Download content for up to `limit` messages that lack it, newest
    /// first: all mail, or the last `download_days`. With a storage limit,
    /// the oldest content is freed to make room for newer mail, and
    /// downloading stops when only older mail is left to free.
    pub(crate) async fn fetch_bodies(&self, ctx: &SyncCtx, mail: &dyn MailProvider, limit: u32) {
        let s = &ctx.settings;
        let since = s.download_days.map(|d| Utc::now() - chrono::Duration::days(d.max(s.initial_sync_days).max(7) as i64));
        let cap = s.storage_limit_mb.map(|mb| mb.saturating_mul(1024 * 1024));
        if let Some(cap) = cap
            && let Err(e) = self.free_space(ctx, cap, None) {
                warn!(account = %ctx.account.email, "freeing space: {e:#}");
            }
        let ids = ctx.store.messages_without_body(&ctx.account.id, since, limit).unwrap_or_default();
        let total = ids.len();
        for (i, (id, date, size)) in ids.into_iter().enumerate() {
            if let Some(cap) = cap {
                let need = if size == 0 { UNKNOWN_SIZE } else { size.min(MAX_ESTIMATE) };
                match self.free_space(ctx, cap.saturating_sub(need), Some(date)) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => {
                        warn!(account = %ctx.account.email, "freeing space: {e:#}");
                        break;
                    }
                }
            }
            if i % 25 == 0 {
                ctx.progress(format!("Downloading messages {}/{}", i, total));
            }
            if let Err(e) = self.ensure_body(ctx, mail, &id).await {
                debug!("body {id}: {e:#}");
            }
        }
    }

    /// Free the account's oldest stored content (dated before `before`, if
    /// given) until it takes at most `target` bytes. Returns false if it
    /// can't get there.
    pub(crate) fn free_space(&self, ctx: &SyncCtx, target: u64, before: Option<DateTime<Utc>>) -> Result<bool> {
        let mut used = ctx.store.stored_bytes(&ctx.account.id)?;
        while used > target {
            let Some((id, hash, size)) = ctx.store.oldest_with_body(&ctx.account.id, before)? else { return Ok(false) };
            ctx.store.drop_blob(&id)?;
            if !ctx.store.blob_in_use(&hash)? {
                self.blobs.delete(&hash)?;
            }
            used = used.saturating_sub(size);
        }
        Ok(true)
    }

    /// The message's raw bytes: the stored copy, or downloaded and stored
    /// without attachment contents, which stay on the server where the
    /// provider can leave them out. A download returns what came down: the
    /// full message, or the rebuilt copy without attachment contents.
    pub(crate) async fn ensure_body(&self, ctx: &SyncCtx, mail: &dyn MailProvider, id: &str) -> Result<Option<Vec<u8>>> {
        let Some(r) = ctx.store.message_ref(id)? else { return Ok(None) };
        if let Some(h) = &r.blob_hash
            && let Some(b) = self.blobs.get(h)? {
                return Ok(Some(b));
            }
        let whole = ctx.keeps_attachments() || (r.size > 0 && r.size < satchel_mime::SLIM_FROM_BYTES);
        let d = if whole { Download::Full(mail.fetch_raw(ctx, &r).await?) } else { mail.fetch_without_attachments(ctx, &r).await? };
        let (hash, size, partial, parsed) = ctx.put_download(&d)?;
        self.set_body(ctx, id, r.blob_hash.as_deref(), &StoredBlob { hash: &hash, size, partial, snippet: &parsed.snippet, attachments: &parsed.attachments })?;
        Ok(Some(match d {
            Download::Full(raw) => raw,
            Download::Slim(s) => s.raw,
        }))
    }

    /// The full message, attachments included, downloading it if the stored
    /// copy leaves them out. Kept until the storage limit frees it.
    pub(crate) async fn ensure_full(&self, ctx: &SyncCtx, mail: &dyn MailProvider, id: &str) -> Result<Vec<u8>> {
        let r = ctx.store.message_ref(id)?.ok_or_else(|| crate::error::not_found("message"))?;
        let raw = mail.fetch_raw(ctx, &r).await?;
        self.store_body(ctx, id, r.blob_hash.as_deref(), &raw, true)?;
        Ok(raw)
    }

    fn store_body(&self, ctx: &SyncCtx, id: &str, old: Option<&str>, raw: &[u8], full: bool) -> Result<()> {
        let parsed = satchel_mime::parse(raw).unwrap_or_default();
        let (hash, size, partial) = ctx.put_blob(raw, full)?;
        self.set_body(ctx, id, old, &StoredBlob { hash: &hash, size, partial, snippet: &parsed.snippet, attachments: &parsed.attachments })
    }

    fn set_body(&self, ctx: &SyncCtx, id: &str, old: Option<&str>, b: &StoredBlob) -> Result<()> {
        ctx.store.set_blob(id, b)?;
        if let Some(old) = old.filter(|o| *o != b.hash)
            && !ctx.store.blob_in_use(old)? {
                self.blobs.delete(old)?;
            }
        Ok(())
    }
}
