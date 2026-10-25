//! Background workers: the search indexer and the AI folder writer.

use super::*;
use std::time::Duration;

use anyhow::Result;
use tracing::{debug, warn};


impl Engine {
    pub(crate) async fn indexer(this: std::sync::Weak<Self>) {
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let Some(e) = this.upgrade() else { return };
            // Drain the backlog in batches; only sleep once caught up.
            loop {
                let e2 = e.clone();
                match tokio::task::spawn_blocking(move || e2.index_batch(1000)).await {
                    Ok(Ok(1000)) => continue,
                    Ok(Err(err)) => warn!("indexer: {err:#}"),
                    _ => {}
                }
                break;
            }
        }
    }

    pub fn index_batch(&self, n: u32) -> Result<usize> {
        let ids = self.store.unindexed(n)?;
        if ids.is_empty() {
            return Ok(0);
        }
        let mut docs = vec![];
        for id in &ids {
            let Some(m) = self.store.message_row(id)? else { continue };
            let raw = m.blob_hash.as_ref().and_then(|h| self.blobs.get(h).ok().flatten());
            let body = raw.as_deref().and_then(satchel_mime::parse).map(|p| p.text).unwrap_or_else(|| m.summary.snippet.clone());
            let participants = m.summary.from.iter().chain(m.summary.to.iter()).chain(m.cc.iter()).map(|a| a.display()).collect::<Vec<_>>().join(" ");
            docs.push(crate::store::SearchDoc {
                id: m.summary.id.clone(),
                subject: m.summary.subject.clone(),
                participants,
                body,
                attachments: m.attachments.iter().map(|a| a.filename.clone()).collect::<Vec<_>>().join(" "),
                blob_hash: m.blob_hash.clone(),
            });
        }
        self.store.index_messages(&docs)?;
        Ok(ids.len())
    }

    pub(crate) async fn mirror_worker(this: std::sync::Weak<Self>) {
        let mut last_index = std::time::Instant::now() - Duration::from_secs(3600);
        loop {
            let wake = match this.upgrade() {
                Some(e) => e.mirror_wake.clone(),
                None => return,
            };
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                _ = wake.notified() => {},
            }
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            let do_indexes = last_index.elapsed() > Duration::from_secs(20);
            let r = tokio::task::spawn_blocking(move || e2.mirror_pass(do_indexes)).await;
            match r {
                Ok(Ok(wrote)) => {
                    if do_indexes {
                        last_index = std::time::Instant::now();
                    }
                    if wrote > 0 {
                        debug!("mirror: wrote {wrote} threads");
                    }
                }
                Ok(Err(err)) => warn!("mirror: {err:#}"),
                Err(_) => {}
            }
            e.ingest_dropped_proposals().await;
        }
    }

    /// Keeps automatic contacts current: once at startup (the backfill over
    /// existing mail), then after syncs, at most every half minute.
    pub(crate) async fn contacts_worker(this: std::sync::Weak<Self>) {
        loop {
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            match tokio::task::spawn_blocking(move || e2.contacts_pass()).await {
                Ok(Ok(n)) if n > 0 => debug!("contacts: {n} created or updated"),
                Ok(Err(err)) => warn!("contacts: {err:#}"),
                _ => {}
            }
            let wake = e.contacts_wake.clone();
            drop(e);
            tokio::time::sleep(Duration::from_secs(30)).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(600)) => {},
                _ = wake.notified() => {},
            }
        }
    }

    /// One incremental pass of the AI mirror. Returns threads written.
    pub fn mirror_pass(&self, indexes: bool) -> Result<usize> {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return Ok(0);
        }
        let m = crate::ai_folder::Mirror { store: &self.store, blobs: self.blobs.as_ref(), settings: &settings.mirror, contact_files: &self.contact_files };
        m.ensure_layout()?;
        let mut n = 0;
        loop {
            let threads = self.store.unmirrored_threads(200)?;
            if threads.is_empty() {
                break;
            }
            for t in &threads {
                if m.write_thread(t)? {
                    n += 1;
                }
                self.store.mark_thread_mirrored(t)?;
            }
        }
        if indexes || n > 0 {
            m.cleanup_orphans()?;
            m.write_indexes()?;
            m.write_calendar()?;
        }
        if indexes || n > 0 || self.contacts_dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
            m.write_contacts()?;
        }
        Ok(n)
    }
}
