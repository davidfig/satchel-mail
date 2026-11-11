//! Settings sync between computers through a shared folder (0080).
//!
//! - `crypto.rs` — the key, the sync code, sealing each computer's file.
//! - `folder.rs` — the header and the per-computer files on disk.
//! - `state.rs` — the last-writer-wins merge and finding local changes.
//! - `items.rs` — what syncs, and applying another computer's items here.

mod crypto;
mod folder;
mod items;
mod proposals;
mod state;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use satchel_api::*;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::Engine;
use crate::store::{ms_to_dt, now_ms};
use state::Known;

const CONFIG: &str = "sync.config";
const KNOWN: &str = "sync.known";
const STATUS: &str = "sync.status";
const SECRET: &str = "sync:key";

/// How often the folder is looked at.
const EVERY: Duration = Duration::from_secs(15);

/// This computer's part of the sync. Not itself synced.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    folder: String,
    device_id: String,
    device_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Status {
    last_pass: Option<i64>,
    /// This computer's accounts (emails) at the last pass, to see new ones.
    #[serde(default)]
    accounts: Vec<String>,
    error: Option<String>,
    devices: Vec<SyncDevice>,
}

/// Passes never overlap, in this process.
static PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl Engine {
    fn sync_config(&self) -> Result<Option<Config>> {
        self.store.get_setting(CONFIG)
    }

    pub(crate) fn sync_config_exists(&self) -> Result<bool> {
        Ok(self.sync_config()?.is_some())
    }

    fn sync_key(&self) -> Result<Option<crypto::Key>> {
        let Some(Credential::Password { password }) = self.secrets.get(SECRET)? else { return Ok(None) };
        Ok(B64.decode(password)?.try_into().ok())
    }

    pub(crate) fn sync_status(&self) -> Result<FolderSync> {
        let Some(cfg) = self.sync_config()? else { return Ok(FolderSync::default()) };
        let st: Status = self.store.get_setting(STATUS)?.unwrap_or_default();
        let known: Known = self.store.get_setting(KNOWN)?.unwrap_or_default();
        let here: BTreeSet<String> = self.store.list_accounts()?.into_iter().map(|a| a.email.to_lowercase()).collect();
        let offered_accounts = known
            .items
            .iter()
            .filter(|(k, (e, _))| k.strip_prefix("account:").is_some_and(|m| !here.contains(m)) && !e.v.is_null())
            .filter_map(|(_, (e, _))| {
                Some(OfferedAccount {
                    config: serde_json::from_value(e.v["config"].clone()).ok()?,
                    color: e.v["color"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect();
        Ok(FolderSync {
            enabled: true,
            folder: Some(cfg.folder),
            device_id: Some(cfg.device_id),
            device_name: Some(cfg.device_name),
            devices: st.devices,
            last_pass: st.last_pass.map(ms_to_dt),
            error: st.error,
            offered_accounts,
        })
    }

    /// Starts a new sync in `folder`. Returns the sync code.
    pub(crate) async fn enable_sync(self: &Arc<Self>, folder: &str) -> Result<String> {
        let _g = PASS.lock().await;
        let dir = sync_dir(folder)?;
        if self.sync_config()?.is_some() {
            bail!("sync is already on; turn it off first");
        }
        if folder::read_header(&dir)?.is_some() {
            bail!("that folder already holds a Satchel sync: join it with the sync code from another computer");
        }
        let key = crypto::new_key()?;
        folder::write_header(&dir, &key)?;
        self.start_sync(&dir, &key, now_ms())?;
        Ok(crypto::code(&key))
    }

    /// Joins the sync in `folder`. What's already there wins over this
    /// computer's settings; anything only this computer has is added.
    pub(crate) async fn join_sync(self: &Arc<Self>, folder: &str, code: &str) -> Result<()> {
        {
            let _g = PASS.lock().await;
            let dir = sync_dir(folder)?;
            if self.sync_config()?.is_some() {
                bail!("sync is already on; turn it off first");
            }
            let key = crypto::parse_code(code)?;
            match folder::read_header(&dir)? {
                None => bail!("that folder doesn't hold a Satchel sync yet: turn sync on there instead"),
                Some(check) if check != crypto::key_check(&key) => bail!("that isn't the sync code for this folder"),
                Some(_) => {}
            }
            // Stamped at time zero, so every other computer's version wins.
            self.start_sync(&dir, &key, 0)?;
        }
        self.sync_pass().await
    }

    fn start_sync(&self, dir: &std::path::Path, key: &crypto::Key, stamp: i64) -> Result<()> {
        let cfg = Config {
            folder: dir.to_string_lossy().into(),
            device_id: format!("device-{}", crate::random_token(9).replace(['-', '_'], "x")),
            device_name: device_name(),
        };
        self.secrets.set(SECRET, &Credential::Password { password: B64.encode(key) })?;
        let mut known = Known::default();
        known.take_local(&items::local(self)?, stamp, &cfg.device_id);
        self.write_own(dir, key, &cfg, &known)?;
        self.store.set_setting(KNOWN, &known)?;
        self.store.set_setting(CONFIG, &cfg)?;
        self.store.set_setting(STATUS, &Status { last_pass: Some(now_ms()), ..Default::default() })?;
        self.store.audit("user", "sync_on", &serde_json::json!({ "folder": cfg.folder }))?;
        Ok(())
    }

    /// Stops syncing here and takes this computer's file out of the folder.
    /// The other computers keep what they have.
    pub(crate) async fn disable_sync(&self) -> Result<()> {
        let _g = PASS.lock().await;
        let Some(cfg) = self.sync_config()? else { return Ok(()) };
        if let Err(e) = folder::remove_snapshot(&PathBuf::from(&cfg.folder), &cfg.device_id) {
            warn!("sync: couldn't remove this computer's file: {e:#}");
        }
        self.secrets.delete(SECRET)?;
        for k in [CONFIG, KNOWN, STATUS] {
            self.store.delete_setting(k)?;
        }
        self.store.audit("user", "sync_off", &serde_json::json!({ "folder": cfg.folder }))?;
        Ok(())
    }

    /// One pass: find what changed here, take in the other computers'
    /// changes, and write this computer's file if anything moved.
    pub(crate) async fn sync_pass(self: &Arc<Self>) -> Result<()> {
        let _g = PASS.lock().await;
        let Some(cfg) = self.sync_config()? else { return Ok(()) };
        let mut st: Status = self.store.get_setting(STATUS)?.unwrap_or_default();
        let result = self.pass_inner(&cfg, &mut st).await;
        st.last_pass = Some(now_ms());
        st.error = result.as_ref().err().map(|e| format!("{e:#}"));
        self.store.set_setting(STATUS, &st)?;
        result
    }

    async fn pass_inner(self: &Arc<Self>, cfg: &Config, st: &mut Status) -> Result<()> {
        let key = self.sync_key()?.ok_or_else(|| anyhow!("this computer has lost its sync code: turn sync off and join again"))?;
        let dir = PathBuf::from(&cfg.folder);
        match folder::read_header(&dir) {
            Ok(Some(c)) if c == crypto::key_check(&key) => {}
            Ok(Some(_)) => bail!("sync was started again in this folder with another code: turn sync off and join with the new code"),
            Ok(None) => bail!("the sync folder is missing or empty ({})", dir.display()),
            Err(e) => return Err(e),
        }
        let mut known: Known = self.store.get_setting(KNOWN)?.unwrap_or_default();
        let before = known.entries();
        let local = items::local(self)?;
        let here: BTreeSet<String> = self.store.list_accounts()?.into_iter().map(|a| a.email.to_lowercase()).collect();
        for k in known.take_local(&local, now_ms(), &cfg.device_id) {
            if let (true, Some(prev)) = (k.starts_with("agent:"), before.get(&k)) {
                let e = &mut known.items.get_mut(&k).unwrap().0;
                if !e.v.is_null() {
                    items::keep_absent_accounts(&mut e.v, &prev.v, &here);
                }
            }
        }
        let old = proposals::cutoff().timestamp_millis();
        known.forget_old(old, &local);

        let (snaps, problems) = folder::read_others(&dir, &key, &cfg.device_id)?;
        let mut changed = BTreeSet::new();
        for s in &snaps {
            let items = s.items.iter().filter(|(k, e)| state::removable(k) || e.at >= old).map(|(k, e)| (k.clone(), e.clone())).collect();
            changed.extend(known.merge(&items).into_iter().map(|(k, _)| k));
        }
        // An account added here since the last pass: agents given access to
        // it on another computer get that access here too.
        let added: BTreeSet<String> = here.iter().filter(|e| !st.accounts.contains(e)).cloned().collect();
        st.accounts = here.iter().cloned().collect();
        if !added.is_empty() {
            changed.extend(known.items.iter().filter(|(k, (e, _))| k.starts_with("agent:") && items::names_any(&e.v, &added)).map(|(k, _)| k.clone()));
        }
        // Proposals that came before their messages or events did: try again.
        changed.extend(
            known.items.iter().filter(|(k, (e, _))| k.starts_with("proposal:") && !e.v.is_null() && !local.contains_key(*k)).map(|(k, _)| k.clone()),
        );
        let mut failed = vec![];
        let mut outcomes = vec![];
        for k in &changed {
            let v = known.items[k].0.v.clone();
            match items::apply(self, k, &v) {
                Ok(o) => outcomes.push((k.clone(), o)),
                Err(e) => failed.push(format!("{k}: {e:#}")),
            }
        }
        if !outcomes.is_empty() {
            let after = items::local(self)?;
            for (k, o) in &outcomes {
                known.seen(k, if *o == proposals::Outcome::Done { after.get(k) } else { None });
            }
            // A decision kept here over an older one from elsewhere goes out again.
            known.take_local(&after, now_ms(), &cfg.device_id);
            if outcomes.iter().any(|(k, _)| !k.starts_with("proposal:")) {
                self.emit(CoreEvent::AccountsChanged);
            }
        }
        let own = dir.join(format!("{}.json", cfg.device_id));
        if known.entries() != before || !own.exists() {
            self.write_own(&dir, &key, cfg, &known)?;
        }
        self.store.set_setting(KNOWN, &known)?;
        st.devices = snaps
            .iter()
            .map(|s| SyncDevice { id: s.device.clone(), name: s.name.clone(), last_seen: ms_to_dt(s.written_at) })
            .collect();
        let all: Vec<String> = problems.into_iter().chain(failed).collect();
        if !all.is_empty() {
            bail!("{}", all.join("; "));
        }
        Ok(())
    }

    fn write_own(&self, dir: &std::path::Path, key: &crypto::Key, cfg: &Config, known: &Known) -> Result<()> {
        let snap = folder::Snapshot {
            device: cfg.device_id.clone(),
            name: cfg.device_name.clone(),
            written_at: now_ms(),
            items: known.entries(),
        };
        folder::write_snapshot(dir, key, &snap)
    }

    /// Runs a pass every few seconds while sync is on.
    pub(crate) async fn sync_worker(this: Weak<Self>) {
        loop {
            let Some(e) = this.upgrade() else { return };
            if e.sync_config().ok().flatten().is_some()
                && let Err(err) = e.sync_pass().await
            {
                warn!("sync: {err:#}");
            }
            drop(e);
            tokio::time::sleep(EVERY).await;
        }
    }
}

fn sync_dir(folder: &str) -> Result<PathBuf> {
    let p = PathBuf::from(folder.trim());
    if !p.is_absolute() {
        bail!("choose a folder by its full path");
    }
    Ok(p)
}

/// This computer's name, as the others list it: on a Mac the name set in
/// System Settings ("David's MacBook"), elsewhere the host name.
fn device_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output()
        && out.status.success()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname NUL-terminates within it.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]).trim_end_matches(".local").to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This computer".into())
}
