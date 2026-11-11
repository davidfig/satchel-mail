//! Downloading attachments for the AI folder (0078): for messages inside the
//! attachment window whose stored copy leaves them out, newest first, in the
//! background. They're written only to the folder; the stored copy stays
//! slim, so each attachment is on disk once.

use super::*;

use std::collections::HashSet;

use tracing::debug;

use crate::provider::priority;

/// Messages looked at per round, and the pause when there's nothing to do.
const BATCH: u32 = 20;
const IDLE: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

impl Engine {
    pub(crate) async fn ai_attachment_worker(this: std::sync::Weak<Self>) {
        // Messages that failed this run (offline, gone from the server, a
        // part that won't decode); tried again after a restart.
        let mut failed: HashSet<String> = HashSet::new();
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            let Some(e) = this.upgrade() else { return };
            let done = e.fetch_ai_attachments(&mut failed).await;
            drop(e);
            tokio::time::sleep(if done == 0 { IDLE } else { Duration::from_millis(200) }).await;
        }
    }

    /// One round: write the attachments of up to [`BATCH`] messages that
    /// want them. Returns how many messages it settled.
    pub async fn fetch_ai_attachments(&self, failed: &mut HashSet<String>) -> usize {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return 0;
        }
        let m = crate::ai_folder::Mirror { store: &self.store, blobs: self.blobs.as_ref(), settings: &settings.mirror, contact_files: &self.contact_files, history_seen: &self.history_seen };
        let Some(since) = m.attachment_window() else { return 0 };
        let wanted = match self.store.ai_attachments_wanted(since.map(|d| d.timestamp_millis()), BATCH + failed.len() as u32) {
            Ok(w) => w,
            Err(e) => {
                warn!("AI folder attachments: {e:#}");
                return 0;
            }
        };
        let mut done = 0;
        let wanted: Vec<_> = wanted.into_iter().filter(|(id, ..)| !failed.contains(id)).take(BATCH as usize).collect();
        for (id, thread, account) in wanted {
            match self.fetch_ai_attachment(&settings.mirror, &id, &thread, &account).await {
                Ok(true) => done += 1,
                Ok(false) => {
                    failed.insert(id);
                }
                Err(e) => {
                    debug!("AI folder attachments of {id}: {e:#}");
                    failed.insert(id);
                }
            }
        }
        done
    }

    /// Write one message's attachments to the folder. Returns false when it
    /// can't now (its account isn't shared, or its thread isn't written).
    async fn fetch_ai_attachment(&self, settings: &MirrorSettings, id: &str, thread: &str, account: &str) -> Result<bool> {
        let m = crate::ai_folder::Mirror { store: &self.store, blobs: self.blobs.as_ref(), settings, contact_files: &self.contact_files, history_seen: &self.history_seen };
        if !m.account_allowed(account) {
            return Ok(false);
        }
        let Some(row) = self.store.message_row(id)? else { return Ok(false) };
        let wanted = row.attachments.iter().any(|a| crate::ai_folder::listed_attachment(a) && a.size <= crate::ai_folder::MAX_ATTACHMENT);
        let stored = if row.partial { None } else { row.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten()) };
        let raw = match stored {
            _ if !wanted => None,
            Some(raw) => Some(raw),
            None => {
                // From the server, as the user opening it would, but kept
                // only in the folder.
                let r = self.store.message_ref(id)?.ok_or_else(|| crate::error::not_found("message"))?;
                let providers = self.providers_for(account)?;
                let Some(mail) = providers.mail.clone() else { return Ok(false) };
                let ctx = self.ctx(account)?;
                let gate = self.gate(account);
                gate.wait_for_users().await;
                let fetch = priority::in_background(gate, async { mail.fetch_raw(&ctx, &r).await });
                Some(tokio::time::timeout(FETCH_TIMEOUT, fetch).await.map_err(|_| anyhow::anyhow!("timed out"))??)
            }
        };
        // Written under the folder's lock, so a pass rewriting the thread
        // can't remove the files between writing them and recording it.
        let _writing = self.mirror_lock.lock().unwrap();
        let ok = match &raw {
            Some(raw) => m.write_attachments(thread, id, raw)?,
            None => true,
        };
        if ok {
            self.store.set_ai_attachments(&[id.to_string()], true)?;
        }
        Ok(ok)
    }
}
