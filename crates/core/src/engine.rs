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
    /// Owns sync, the op queue, the search indexer and the mirror.
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
    pub(crate) factory: Arc<dyn ProviderFactory>,
    pub(crate) events: broadcast::Sender<CoreEvent>,
    pub(crate) runtimes: Mutex<HashMap<String, AccountRuntime>>,
    pub(crate) settings: RwLock<Settings>,
    pub(crate) mirror_wake: Arc<Notify>,
    pub(crate) correspondents: Mutex<Option<(std::time::Instant, Arc<crate::spam::Correspondents>)>>,
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
        // Search moved from a tantivy index to SQLite FTS5: drop the old
        // directory and build the new index once.
        let legacy = opts.data_dir.join("index");
        if opts.mode == Mode::Full && legacy.exists() {
            let _ = std::fs::remove_dir_all(&legacy);
        }
        if opts.mode == Mode::Full && store.get_setting::<String>("search_index")?.as_deref() != Some("fts5-v1") {
            store.reset_indexed()?;
            store.set_setting("search_index", &"fts5-v1")?;
        }
        Self::with_parts(store, blobs, opts)
    }

    pub fn with_parts(store: Arc<Store>, blobs: Arc<dyn BlobStore>, opts: EngineOptions) -> Result<Arc<Self>> {
        let settings: Settings = store.get_setting("settings")?.unwrap_or_default();
        let (events, _) = broadcast::channel(1024);
        let engine = Arc::new(Self {
            store,
            blobs,
            secrets: opts.secrets,
            factory: opts.factory,
            events,
            runtimes: Mutex::new(HashMap::new()),
            settings: RwLock::new(settings),
            mirror_wake: Arc::new(Notify::new()),
            correspondents: Mutex::new(None),
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
        Ok(SyncCtx { account, store: self.store.clone(), blobs: self.blobs.clone(), settings: self.settings(), events: self.events.clone(), arrivals: Default::default() })
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
                    // The user's sender rules outrank the server's spam filter.
                    if let Err(e) = engine.enforce_sender_rules(&account_id) {
                        warn!(account = %ctx.account.email, "sender rules: {e:#}");
                    }
                    engine.fetch_bodies(&ctx, mail.as_ref(), if first { 300 } else { 100 }).await;
                }
            }
            if let Some(cal) = &providers.calendar {
                if let Err(e) = cal.sync(&ctx).await {
                    warn!(account = %ctx.account.email, "calendar sync: {e:#}");
                    if result.is_ok() {
                        result = Err(e);
                    }
                } else if let Err(e) = engine.remove_blocked_invites(&account_id) {
                    warn!(account = %ctx.account.email, "spam invites: {e:#}");
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

    /// Turn files in the AI folder's `proposals/inbox/` into proposals.
    pub async fn ingest_dropped_proposals(self: &Arc<Self>) {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return;
        }
        for path in crate::proposal_file::pending(&settings.mirror) {
            let created = match crate::proposal_file::read(&self.store, &path) {
                Ok(d) => self.ingest_one(&settings.mirror, d).await,
                Err(e) => Err(format!("{e:#}")),
            };
            let outcome = match &created {
                Ok(p) => crate::proposal_file::Outcome::Created(p),
                Err(e) => crate::proposal_file::Outcome::Error(e.clone()),
            };
            crate::proposal_file::finish(&settings.mirror, &path, outcome);
        }
    }

    async fn ingest_one(self: &Arc<Self>, mirror: &MirrorSettings, d: crate::proposal_file::DroppedProposal) -> std::result::Result<Proposal, String> {
        let agent = match &d.agent_token {
            Some(t) => self.authenticate_agent(t).ok_or("invalid or revoked agent_token")?,
            None => self.folder_agent(mirror).map_err(|e| format!("{e:#}"))?,
        };
        let rationale = match d.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(label) => format!("[{label}] {}", d.rationale),
            None => d.rationale,
        };
        let cmd = Command::CreateProposal { action: d.action, rationale, source_thread_ids: d.source_thread_ids };
        let v = self.handle(&Caller::Agent(agent), cmd).await.map_err(|e| e.message)?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// The built-in agent behind token-less proposal files. What it can see
    /// follows the AI folder settings; its rate limit is the default policy's.
    fn folder_agent(&self, mirror: &MirrorSettings) -> Result<Agent> {
        if !mirror.accept_proposals {
            anyhow::bail!("proposals from the AI folder are turned off (Settings → AI folder); include an agent_token to use a connected agent");
        }
        let mut a = self.store.builtin_agent("ai-folder", "AI folder", &AgentPolicy::default())?;
        a.policy.account_ids = mirror.account_ids.clone();
        a.policy.since_days = mirror.since_days;
        a.policy.allow_attachments = mirror.include_attachments;
        let _ = self.store.touch_agent(&a.id);
        Ok(a)
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
