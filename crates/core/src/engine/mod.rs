//! The running core: owns storage, provider runtimes and background workers,
//! and executes [`Command`]s for a [`Caller`].

mod account_loop;
mod bodies;
mod workers;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use satchel_api::*;
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;
use tracing::{info, warn};

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
    pub(crate) contacts_wake: Arc<Notify>,
    /// Accounts whose address books sync on the next pass ("Sync now").
    pub(crate) contacts_due: Mutex<std::collections::HashSet<String>>,
    pub(crate) correspondents: Mutex<Option<(std::time::Instant, Arc<crate::mail::spam::Correspondents>)>>,
    /// The AI folder's person files as last written (see `Mirror`).
    pub(crate) contact_files: Mutex<HashMap<String, String>>,
    /// Contacts changed since the AI folder last wrote them.
    pub(crate) contacts_dirty: std::sync::atomic::AtomicBool,
    pub mode: Mode,
    pub data_dir: PathBuf,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

/// How often address books are synced.
const CONTACTS_EVERY: Duration = Duration::from_secs(300);

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
            contacts_wake: Arc::new(Notify::new()),
            contacts_due: Mutex::new(Default::default()),
            correspondents: Mutex::new(None),
            contact_files: Mutex::new(HashMap::new()),
            contacts_dirty: std::sync::atomic::AtomicBool::new(true),
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
        if matches!(e, CoreEvent::ContactsChanged) {
            self.contacts_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
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
        w.push(tokio::spawn(Self::contacts_worker(Arc::downgrade(self))));
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
