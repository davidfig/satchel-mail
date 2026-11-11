//! Background workers: the search indexer, the AI folder writer, deleting
//! unused stored mail, calendar alerts and repairing old snippets.

use super::*;
use std::time::Duration;

use anyhow::Result;
use tracing::{debug, warn};

/// Messages indexed per batch, and the most body text indexed per message:
/// a batch holds every body in memory at once.
const INDEX_BATCH: u32 = 200;
const MAX_INDEXED_BODY: usize = 1024 * 1024;

/// How often `smart-folders.md` is rewritten when nothing else prompts it.
const SMART_FOLDERS_REFRESH_MS: i64 = 10 * 60 * 1000;
/// How often every thread folder is checked for ones that went missing.
const MIRROR_CHECK_MS: i64 = 24 * 60 * 60 * 1000;
/// How often every part of the AI folder is written whatever its stamp says,
/// in case its files were changed or deleted behind Satchel's back.
const MIRROR_REFRESH_MS: i64 = 10 * 60 * 1000;

impl Engine {
    pub(crate) async fn indexer(this: std::sync::Weak<Self>) {
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let Some(e) = this.upgrade() else { return };
            // Drain the backlog in batches; only sleep once caught up.
            loop {
                let e2 = e.clone();
                match tokio::task::spawn_blocking(move || crate::store::background(|| e2.index_batch(INDEX_BATCH))).await {
                    Ok(Ok(n)) if n == INDEX_BATCH as usize => continue,
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
            let mut body = raw.as_deref().and_then(satchel_mime::parse).map(|p| p.text).unwrap_or_else(|| m.summary.snippet.clone());
            if body.len() > MAX_INDEXED_BODY {
                body.truncate(body.floor_char_boundary(MAX_INDEXED_BODY));
            }
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
            let r = tokio::task::spawn_blocking(move || crate::store::background(|| e2.mirror_pass(do_indexes))).await;
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
            match tokio::task::spawn_blocking(move || crate::store::background(|| e2.contacts_pass())).await {
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

    /// Deletes stored copies no message uses: at startup every one an
    /// earlier run left behind, then those of messages deleted since.
    pub(crate) async fn blob_worker(this: std::sync::Weak<Self>) {
        let mut swept = false;
        loop {
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            let sweep = !swept;
            let r = tokio::task::spawn_blocking(move || crate::store::background(|| if sweep { e2.sweep_orphan_blobs() } else { e2.free_unused_blobs() })).await;
            match r {
                Ok(Ok(n)) => {
                    swept = true;
                    if n > 0 {
                        debug!("blobs: deleted {n} unused");
                    }
                }
                Ok(Err(err)) => warn!("blobs: {err:#}"),
                Err(_) => {}
            }
            drop(e);
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    /// Sends calendar alerts as they come due (0056): sleeps until the next
    /// one, but at most a minute (the clock may jump, as when the computer
    /// wakes), and wakes early when calendars, settings or alerts change.
    pub(crate) async fn alerts_worker(this: std::sync::Weak<Self>) {
        loop {
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            let mut wait = Duration::from_secs(60);
            match tokio::task::spawn_blocking(move || crate::store::background(|| e2.alerts_pass(chrono::Utc::now()))).await {
                Ok(Ok((due, next))) => {
                    if !due.is_empty() {
                        e.emit(CoreEvent::AlertsChanged { due, closed: vec![] });
                    }
                    if let Some(n) = next.and_then(|n| (n - chrono::Utc::now()).to_std().ok()) {
                        wait = wait.min(n + Duration::from_millis(200));
                    }
                }
                Ok(Err(err)) => warn!("alerts: {err:#}"),
                Err(_) => {}
            }
            let wake = e.alerts_wake.clone();
            drop(e);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                _ = wake.notified() => {},
            }
        }
    }

    /// Once per database, rebuilds snippets stored before they decoded HTML
    /// character references. Tried again next start if it fails.
    pub(crate) async fn snippet_repair_worker(this: std::sync::Weak<Self>) {
        const KEY: &str = "snippet_repair";
        const DONE: &str = "entities-v1";
        let Some(e) = this.upgrade() else { return };
        let e2 = e.clone();
        let r = tokio::task::spawn_blocking(move || {
            crate::store::background(|| {
                if e2.store.get_setting::<String>(KEY)?.as_deref() == Some(DONE) {
                    return Ok(0);
                }
                let n = e2.repair_snippets()?;
                e2.store.set_setting(KEY, &DONE)?;
                Ok::<_, anyhow::Error>(n)
            })
        })
        .await;
        match r {
            Ok(Ok(n)) if n > 0 => {
                debug!("snippets: repaired {n}");
                e.emit(CoreEvent::DataChanged);
            }
            Ok(Err(err)) => warn!("snippets: {err:#}"),
            _ => {}
        }
    }

    /// Rebuild, from the stored copy, each snippet that may hold a literal
    /// character reference. Returns how many changed.
    pub fn repair_snippets(&self) -> Result<usize> {
        let mut n = 0;
        for (id, old, hash) in self.store.snippets_with_entities()? {
            // A copy that's missing or unreadable keeps the snippet it has.
            let Some(parsed) = self.blobs.get(&hash).ok().flatten().as_deref().and_then(satchel_mime::parse) else { continue };
            if parsed.snippet != old {
                self.store.set_snippet(&id, &parsed.snippet)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Delete the stored copies of deleted messages that no message uses
    /// any more. Returns how many were deleted.
    pub fn free_unused_blobs(&self) -> Result<usize> {
        let mut hashes = self.store.take_freed_blobs();
        hashes.sort();
        hashes.dedup();
        let mut n = 0;
        for (i, h) in hashes.iter().enumerate() {
            match self.store.blob_in_use(h) {
                Ok(true) => {}
                Ok(false) => {
                    self.blobs.delete(h)?;
                    n += 1;
                }
                Err(e) => {
                    // Try the rest again next time.
                    self.store.blobs_freed(hashes[i..].iter().cloned());
                    return Err(e);
                }
            }
        }
        Ok(n)
    }

    /// Delete every stored copy no message uses (left by deletes before a
    /// crash or quit, or by older versions). Copies written in the last few
    /// minutes are left alone: their message may not be stored yet.
    pub fn sweep_orphan_blobs(&self) -> Result<usize> {
        let in_use = self.store.blobs_in_use()?;
        let mut n = 0;
        for h in self.blobs.list(Duration::from_secs(600))? {
            if !in_use.contains(&h) && !self.store.blob_in_use(&h)? {
                self.blobs.delete(&h)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Check every AI folder thread folder on the next pass, not just daily.
    pub fn recheck_mirror(&self) {
        self.mirror_check_due.store(1, std::sync::atomic::Ordering::Relaxed);
        self.refresh_mirror();
    }

    /// Write every part of the AI folder on the next pass, not just the parts
    /// whose mail, events or contacts changed (as every ten minutes): for
    /// files changed or deleted behind Satchel's back.
    pub fn refresh_mirror(&self) {
        self.mirror_stamps.lock().unwrap().clear();
        self.mirror_wake.notify_one();
    }

    /// One incremental pass of the AI mirror. Returns threads written.
    pub fn mirror_pass(&self, indexes: bool) -> Result<usize> {
        let _moving = self.mirror_lock.lock().unwrap();
        let settings = self.settings();
        if !settings.mirror.enabled {
            return Ok(0);
        }
        let m = crate::ai_folder::Mirror { store: &self.store, blobs: self.blobs.as_ref(), settings: &settings.mirror, contact_files: &self.contact_files, history_seen: &self.history_seen };
        m.ensure_layout()?;
        // At startup and then daily: find thread folders that went missing,
        // and threads the date limit has moved past, so they're written
        // again (or removed) below. Until a check has run, the folder isn't
        // reported complete.
        let now = crate::store::now_ms();
        let due = self.mirror_check_due.load(std::sync::atomic::Ordering::Relaxed);
        let mut checked = due > 0;
        if due <= now {
            match m.check_threads().and_then(|_| m.sweep_aged()) {
                Ok(_) => {
                    self.mirror_check_due.store(now + MIRROR_CHECK_MS, std::sync::atomic::Ordering::Relaxed);
                    checked = true;
                }
                Err(e) => warn!("mirror: checking thread folders: {e:#}"),
            }
        }
        let mut n = 0;
        let mut first = true;
        loop {
            let threads = self.store.unmirrored_threads(200)?;
            if threads.is_empty() {
                break;
            }
            // Say a big rewrite is under way, so agents don't take a
            // half-written folder for the whole.
            if first && threads.len() >= 200
                && let Err(e) = m.write_status(false, self.store.unmirrored_thread_count()?) {
                    warn!("mirror: sync.json: {e:#}");
                }
            first = false;
            for t in &threads {
                // One thread that can't be written (a name the disk refuses,
                // say) is logged and marked done, so it can't hold up the rest
                // pass after pass; it's tried again when it changes.
                match m.write_thread(t) {
                    Ok(true) => n += 1,
                    Ok(false) => {}
                    Err(e) => warn!("mirror: thread {t}: {e:#}"),
                }
                self.store.mark_thread_mirrored(t)?;
            }
        }
        let log = |what: &str, r: Result<()>| {
            if let Err(e) = r {
                warn!("mirror: {what}: {e:#}");
            }
        };
        // A part is written again only when what it's built from changed
        // (or the date moved on, for date limits and the agenda), and every
        // part every ten minutes; comparing every file each pass kept an idle
        // machine busy. A stamp is recorded only when its part is written.
        let stamps = self.store.stamps()?;
        let settings_json = serde_json::to_string(&settings.mirror)?;
        let (day, hour) = (now / (24 * 60 * 60 * 1000), now / (60 * 60 * 1000));
        let refresh = self.mirror_stamps.lock().unwrap().get("refresh").is_none_or(|&due| due as i64 <= now);
        let fp = |stamp: u64, when: i64| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (stamp, when, &settings_json).hash(&mut h);
            h.finish()
        };
        let changed = |part: &'static str, stamp: u64, when: i64| refresh || self.mirror_stamps.lock().unwrap().get(part) != Some(&fp(stamp, when));
        let record = |part: &'static str, stamp: u64, when: i64| {
            self.mirror_stamps.lock().unwrap().insert(part, fp(stamp, when));
        };
        if indexes || n > 0 || !checked {
            log("removing old threads", m.cleanup_orphans());
            // Thread folders written this pass change what the indexes link to.
            if n > 0 || changed("mail", stamps.mail, day) {
                log("indexes", m.write_indexes());
                log("history", m.write_history().map(|_| ()));
                record("mail", stamps.mail, day);
            }
            if changed("calendar", stamps.calendar, hour) {
                log("calendar", if settings.mirror.include_calendar { m.write_calendar() } else { m.remove_section("calendar") });
                record("calendar", stamps.calendar, hour);
            }
            // Indexes can queue threads whose folders went missing.
            let pending = self.store.unmirrored_thread_count()?;
            log("sync.json", m.write_status(checked && pending == 0, pending));
            // Only when something it shows may have changed: running every
            // folder's search each pass would be steady work on an idle machine.
            // The timer keeps relative dates (`newer_than:7d`) current.
            if n > 0 || pending > 0 || self.smart_folders_due.load(std::sync::atomic::Ordering::Relaxed) <= now {
                self.smart_folders_due.store(now + SMART_FOLDERS_REFRESH_MS, std::sync::atomic::Ordering::Relaxed);
                log("smart folders", m.write_smart_folders());
            }
        }
        // Contacts show message counts, so mail changes them too.
        let contacts_stamp = stamps.contacts ^ stamps.mail;
        if self.contacts_dirty.swap(false, std::sync::atomic::Ordering::Relaxed) || (indexes && changed("contacts", contacts_stamp, day)) {
            log("contacts", if settings.mirror.include_contacts { m.write_contacts() } else { m.remove_section("contacts") });
            record("contacts", contacts_stamp, day);
        }
        if refresh && indexes {
            self.mirror_stamps.lock().unwrap().insert("refresh", (now + MIRROR_REFRESH_MS) as u64);
        }
        Ok(n)
    }
}
