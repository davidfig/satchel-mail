//! The running core: owns storage, provider runtimes and background workers,
//! and executes [`Command`]s for a [`Caller`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use satchel_api::*;
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::blobs::{BlobStore, FsBlobStore};
use crate::provider::{CredentialHandle, ProviderFactory, Providers, SyncCtx};
use crate::search::{IndexDoc, SearchIndex};
use crate::secrets::SecretStore;
use crate::store::Store;

/// Who is issuing a command.
#[derive(Debug, Clone)]
pub enum Caller {
    User,
    Agent(Agent),
}

impl Caller {
    pub fn actor(&self) -> String {
        match self {
            Caller::User => "user".into(),
            Caller::Agent(a) => format!("agent:{}", a.name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Owns sync, the op queue, the search index writer and the mirror.
    Full,
    /// Read + propose only (e.g. an MCP process running beside the app).
    Agent,
}

pub struct EngineOptions {
    pub data_dir: PathBuf,
    pub mode: Mode,
    pub secrets: Arc<dyn SecretStore>,
    pub factory: Arc<dyn ProviderFactory>,
    /// Disable network workers (tests).
    pub start_workers: bool,
}

pub(crate) struct AccountRuntime {
    pub providers: Arc<Providers>,
    pub wake: Arc<Notify>,
    pub task: Option<JoinHandle<()>>,
}

pub struct Engine {
    pub(crate) store: Arc<Store>,
    pub(crate) blobs: Arc<dyn BlobStore>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) search: Arc<SearchIndex>,
    pub(crate) factory: Arc<dyn ProviderFactory>,
    pub(crate) events: broadcast::Sender<CoreEvent>,
    pub(crate) runtimes: Mutex<HashMap<String, AccountRuntime>>,
    pub(crate) settings: RwLock<Settings>,
    pub(crate) mirror_wake: Arc<Notify>,
    pub mode: Mode,
    pub data_dir: PathBuf,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

pub fn default_data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SATCHEL_DATA_DIR") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("app", "Satchel", "Satchel Mail")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".satchel"))
}

impl Engine {
    pub fn open(opts: EngineOptions) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&opts.data_dir)?;
        let store = Arc::new(Store::open(&opts.data_dir.join("satchel.db"))?);
        let blobs: Arc<dyn BlobStore> = Arc::new(FsBlobStore::new(opts.data_dir.join("blobs"))?);
        let search = Arc::new(match SearchIndex::open(&opts.data_dir.join("index"), opts.mode == Mode::Full) {
            Ok(s) => s,
            Err(e) => {
                warn!("search index unavailable ({e}); rebuilding");
                let dir = opts.data_dir.join("index");
                let _ = std::fs::remove_dir_all(&dir);
                store.reset_indexed()?;
                SearchIndex::open(&dir, opts.mode == Mode::Full)?
            }
        });
        Self::with_parts(store, blobs, search, opts)
    }

    pub fn with_parts(store: Arc<Store>, blobs: Arc<dyn BlobStore>, search: Arc<SearchIndex>, opts: EngineOptions) -> Result<Arc<Self>> {
        let settings: Settings = store.get_setting("settings")?.unwrap_or_default();
        let (events, _) = broadcast::channel(1024);
        let engine = Arc::new(Self {
            store,
            blobs,
            secrets: opts.secrets,
            search,
            factory: opts.factory,
            events,
            runtimes: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings),
            mirror_wake: Arc::new(Notify::new()),
            mode: opts.mode,
            data_dir: opts.data_dir,
            workers: Mutex::new(vec![]),
        });
        if opts.start_workers {
            engine.start();
        }
        Ok(engine)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    pub(crate) fn emit(&self, e: CoreEvent) {
        let _ = self.events.send(e);
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    fn start(self: &Arc<Self>) {
        let mut w = self.workers.lock().unwrap();
        w.push(tokio::spawn(Self::data_version_watcher(Arc::downgrade(self))));
        if self.mode != Mode::Full {
            return;
        }
        if let Ok(accounts) = self.store.list_accounts() {
            for a in accounts {
                self.start_account(&a);
            }
        }
        w.push(tokio::spawn(Self::indexer(Arc::downgrade(self))));
        w.push(tokio::spawn(Self::mirror_worker(Arc::downgrade(self))));
    }

    pub fn shutdown(&self) {
        for (_, rt) in self.runtimes.lock().unwrap().drain() {
            if let Some(t) = rt.task {
                t.abort();
            }
        }
        for w in self.workers.lock().unwrap().drain(..) {
            w.abort();
        }
    }

    /// Notices commits from other processes (e.g. an MCP agent adding proposals).
    async fn data_version_watcher(this: std::sync::Weak<Self>) {
        let mut last = None;
        loop {
            tokio::time::sleep(Duration::from_millis(800)).await;
            let Some(e) = this.upgrade() else { return };
            let Ok(v) = e.store.data_version() else { continue };
            if last.is_some_and(|l| l != v) {
                e.emit(CoreEvent::DataChanged);
                e.emit(CoreEvent::ProposalsChanged);
            }
            last = Some(v);
        }
    }

    // -----------------------------------------------------------------------
    // Account runtimes
    // -----------------------------------------------------------------------

    pub(crate) fn providers_for(&self, account_id: &str) -> Result<Arc<Providers>> {
        if let Some(rt) = self.runtimes.lock().unwrap().get(account_id) {
            return Ok(rt.providers.clone());
        }
        let acc = self.store.get_account(account_id)?.ok_or_else(|| crate::error::not_found("account"))?;
        let p = Arc::new(self.factory.create(&acc, CredentialHandle::new(acc.id.clone(), self.secrets.clone()))?);
        self.runtimes.lock().unwrap().insert(account_id.to_string(), AccountRuntime { providers: p.clone(), wake: Arc::new(Notify::new()), task: None });
        Ok(p)
    }

    pub(crate) fn ctx(&self, account_id: &str) -> Result<SyncCtx> {
        let account = self.store.get_account(account_id)?.ok_or_else(|| crate::error::not_found("account"))?;
        Ok(SyncCtx { account, store: self.store.clone(), blobs: self.blobs.clone(), settings: self.settings(), events: self.events.clone() })
    }

    pub(crate) fn start_account(self: &Arc<Self>, acc: &Account) {
        if self.mode != Mode::Full {
            return;
        }
        let providers = match self.factory.create(acc, CredentialHandle::new(acc.id.clone(), self.secrets.clone())) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                warn!(account = %acc.email, "cannot start providers: {e:#}");
                let _ = self.store.set_sync_status(&acc.id, &SyncStatus { state: SyncState::Error, error: Some(format!("{e:#}")), ..Default::default() });
                return;
            }
        };
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(Self::account_loop(Arc::downgrade(self), acc.id.clone(), providers.clone(), wake.clone()));
        if let Some(old) = self.runtimes.lock().unwrap().insert(acc.id.clone(), AccountRuntime { providers, wake, task: Some(task) })
            && let Some(t) = old.task {
                t.abort();
            }
    }

    pub(crate) fn stop_account(&self, account_id: &str) {
        if let Some(rt) = self.runtimes.lock().unwrap().remove(account_id)
            && let Some(t) = rt.task {
                t.abort();
            }
    }

    pub(crate) fn wake_account(&self, account_id: &str) {
        if let Some(rt) = self.runtimes.lock().unwrap().get(account_id) {
            rt.wake.notify_one();
        }
    }

    fn set_status(&self, account_id: &str, st: SyncStatus) {
        let _ = self.store.set_sync_status(account_id, &st);
        self.emit(CoreEvent::SyncStatus { account_id: account_id.to_string(), status: st });
    }

    async fn account_loop(this: std::sync::Weak<Self>, account_id: String, providers: Arc<Providers>, wake: Arc<Notify>) {
        let mut failures: u32 = 0;
        let mut first = true;
        loop {
            let Some(engine) = this.upgrade() else { return };
            let ctx = match engine.ctx(&account_id) {
                Ok(c) => c,
                Err(_) => return,
            };
            let started = std::time::Instant::now();
            let last_success = ctx.account.sync.last_success;
            engine.set_status(&account_id, SyncStatus { state: SyncState::Syncing, last_success, error: None, detail: None });

            let mut result: Result<()> = Ok(());
            // Push local changes first so sync doesn't revert optimistic state.
            if let Err(e) = engine.run_ops(&ctx, &providers).await {
                warn!(account = %ctx.account.email, "ops: {e:#}");
            }
            if let Some(mail) = &providers.mail {
                result = mail.sync(&ctx).await;
                if result.is_ok() {
                    engine.fetch_bodies(&ctx, mail.as_ref(), if first { 300 } else { 100 }).await;
                }
            }
            if let Some(cal) = &providers.calendar {
                if let Err(e) = cal.sync(&ctx).await {
                    warn!(account = %ctx.account.email, "calendar sync: {e:#}");
                    if result.is_ok() {
                        result = Err(e);
                    }
                }
                ctx.calendar_changed();
            }
            ctx.messages_changed();
            engine.mirror_wake.notify_one();
            match &result {
                Ok(()) => {
                    failures = 0;
                    first = false;
                    debug!(account = %ctx.account.email, "sync done in {:?}", started.elapsed());
                    engine.set_status(&account_id, SyncStatus { state: SyncState::Idle, last_success: Some(Utc::now()), error: None, detail: None });
                }
                Err(e) => {
                    failures += 1;
                    let api = crate::error::to_api(anyhow::anyhow!("{e:#}"));
                    let state = if api.code == ErrorCode::Network { SyncState::Offline } else { SyncState::Error };
                    warn!(account = %ctx.account.email, "sync failed: {e:#}");
                    engine.set_status(&account_id, SyncStatus { state, last_success, error: Some(format!("{e:#}")), detail: None });
                }
            }
            let poll = Duration::from_secs(engine.settings().poll_interval_secs.max(15) as u64);
            drop(engine);
            if failures > 0 {
                let backoff = Duration::from_secs((5u64 * 2u64.pow(failures.min(6))).min(600));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {},
                    _ = wake.notified() => {},
                }
                continue;
            }
            match &providers.mail {
                Some(mail) => {
                    let Some(engine) = this.upgrade() else { return };
                    let ctx = match engine.ctx(&account_id) {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    drop(engine);
                    tokio::select! {
                        r = mail.wait_for_changes(&ctx, poll) => { if let Err(e) = r { debug!("wait: {e:#}"); tokio::time::sleep(Duration::from_secs(5)).await; } },
                        _ = wake.notified() => {},
                    }
                }
                None => {
                    tokio::select! {
                        _ = tokio::time::sleep(poll) => {},
                        _ = wake.notified() => {},
                    }
                }
            }
        }
    }

    async fn fetch_bodies(&self, ctx: &SyncCtx, mail: &dyn crate::provider::MailProvider, limit: u32) {
        let since = Utc::now() - chrono::Duration::days(ctx.settings.initial_sync_days.max(7) as i64);
        let ids = ctx.store.messages_without_body(&ctx.account.id, since, limit).unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        let total = ids.len();
        for (i, id) in ids.into_iter().enumerate() {
            if i % 25 == 0 {
                ctx.progress(format!("Downloading messages {}/{}", i, total));
            }
            if let Err(e) = self.ensure_body(ctx, mail, &id).await {
                debug!("body {id}: {e:#}");
            }
        }
    }

    pub(crate) async fn ensure_body(&self, ctx: &SyncCtx, mail: &dyn crate::provider::MailProvider, id: &str) -> Result<Option<Vec<u8>>> {
        let Some(r) = ctx.store.message_ref(id)? else { return Ok(None) };
        if let Some(h) = &r.blob_hash
            && let Some(b) = self.blobs.get(h)? {
                return Ok(Some(b));
            }
        let raw = mail.fetch_raw(ctx, &r).await?;
        let parsed = satchel_mime::parse(&raw).unwrap_or_default();
        let hash = self.blobs.put(&raw)?;
        ctx.store.set_blob(id, &hash, &parsed.snippet, &parsed.attachments)?;
        Ok(Some(raw))
    }

    // -----------------------------------------------------------------------
    // Indexer & mirror workers
    // -----------------------------------------------------------------------

    async fn indexer(this: std::sync::Weak<Self>) {
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            let r = tokio::task::spawn_blocking(move || e2.index_batch(400)).await;
            if let Ok(Err(err)) = r {
                warn!("indexer: {err:#}");
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
            let body = raw.as_deref().and_then(satchel_mime::parse).map(|p| p.text).unwrap_or_default();
            let from = m.summary.from.as_ref().map(|a| a.display()).unwrap_or_default();
            let to = m.summary.to.iter().chain(m.cc.iter()).map(|a| a.display()).collect::<Vec<_>>().join(" ");
            let atts = m.attachments.iter().map(|a| a.filename.clone()).collect::<Vec<_>>().join(" ");
            docs.push((m.summary.id.clone(), m.summary.account_id.clone(), m.summary.subject.clone(), from, to, body, atts));
        }
        let idocs: Vec<IndexDoc> = docs
            .iter()
            .map(|d| IndexDoc { id: &d.0, account_id: &d.1, subject: &d.2, from: &d.3, to: &d.4, body: &d.5, attachment_names: &d.6 })
            .collect();
        self.search.add(&idocs)?;
        self.search.commit()?;
        self.store.mark_indexed(&ids)?;
        Ok(ids.len())
    }

    async fn mirror_worker(this: std::sync::Weak<Self>) {
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

    /// One incremental pass of the AI mirror. Returns threads written.
    pub fn mirror_pass(&self, indexes: bool) -> Result<usize> {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return Ok(0);
        }
        let m = crate::mirror::Mirror { store: &self.store, blobs: self.blobs.as_ref(), settings: &settings.mirror };
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
        Ok(n)
    }

    async fn ingest_dropped_proposals(self: &Arc<Self>) {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return;
        }
        for path in crate::mirror::pending_proposal_files(&settings.mirror) {
            let result = match std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|s| serde_json::from_str::<crate::mirror::DroppedProposal>(&s).map_err(anyhow::Error::from))
            {
                Ok(d) => match self.authenticate_agent(&d.agent_token) {
                    Some(agent) => {
                        let cmd = Command::CreateProposal { action: d.action, rationale: d.rationale, source_thread_ids: d.source_thread_ids };
                        match self.handle(&Caller::Agent(agent), cmd).await {
                            Ok(v) => serde_json::json!({ "ok": v }),
                            Err(e) => serde_json::json!({ "error": e }),
                        }
                    }
                    None => serde_json::json!({ "error": "invalid or revoked agent_token" }),
                },
                Err(e) => serde_json::json!({ "error": format!("unreadable proposal file: {e}") }),
            };
            crate::mirror::finish_proposal_file(&settings.mirror, &path, &result);
        }
    }

    // -----------------------------------------------------------------------
    // Agents
    // -----------------------------------------------------------------------

    pub fn hash_token(token: &str) -> String {
        blake3::hash(token.as_bytes()).to_hex().to_string()
    }

    pub fn authenticate_agent(&self, token: &str) -> Option<Agent> {
        let a = self.store.agent_by_token_hash(&Self::hash_token(token.trim())).ok().flatten()?;
        let _ = self.store.touch_agent(&a.id);
        Some(a)
    }

    pub fn data_path(&self) -> &Path {
        &self.data_dir
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        info!("engine shutting down");
        self.shutdown();
    }
}
