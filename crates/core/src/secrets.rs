//! Credential storage. Desktop uses the OS keychain; tests and headless dev
//! use a 0600 JSON file. Secrets never touch the main database.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use satchel_api::Credential;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Credential>>;
    fn set(&self, key: &str, value: &Credential) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub fn account_key(account_id: &str) -> String {
    format!("account:{account_id}")
}

/// OS keychain store. Values are cached after the first read: macOS asks the
/// user before each access by a binary it doesn't trust yet (every unsigned
/// dev rebuild), so we touch the keychain at most once per item per run.
#[cfg(feature = "keychain")]
pub struct KeychainSecretStore {
    service: String,
    /// Serialized JSON as stored; `None` remembers a missing entry.
    cache: Mutex<HashMap<String, Option<String>>>,
}

#[cfg(feature = "keychain")]
impl KeychainSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into(), cache: Mutex::new(HashMap::new()) }
    }
    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(|e| anyhow!("keychain: {e}"))
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeychainSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        let mut cache = self.cache.lock().unwrap();
        let stored = match cache.get(key) {
            Some(v) => v.clone(),
            None => {
                let v = match self.entry(key)?.get_password() {
                    Ok(s) => Some(s),
                    Err(keyring::Error::NoEntry) => None,
                    Err(e) => return Err(anyhow!("keychain: {e}")),
                };
                cache.insert(key.to_string(), v.clone());
                v
            }
        };
        Ok(stored.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        let json = serde_json::to_string(value)?;
        let mut cache = self.cache.lock().unwrap();
        // Rewriting an unchanged item would still prompt.
        if cache.get(key).is_some_and(|v| v.as_deref() == Some(json.as_str())) {
            return Ok(());
        }
        self.entry(key)?.set_password(&json).map_err(|e| anyhow!("keychain: {e}"))?;
        cache.insert(key.to_string(), Some(json));
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut cache = self.cache.lock().unwrap();
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(anyhow!("keychain: {e}")),
        }
        cache.insert(key.to_string(), None);
        Ok(())
    }
}

/// JSON file store (permissions 0600). Used for tests, CI and headless servers.
pub struct FileSecretStore {
    path: PathBuf,
    cache: Mutex<HashMap<String, Credential>>,
}

impl FileSecretStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        let cache = load(&path)?;
        Ok(Self { path, cache: Mutex::new(cache) })
    }

    /// Writes a fresh 0600 temp file (never one left behind or planted),
    /// syncs it and renames it over the store.
    fn flush(&self, map: &HashMap<String, Credential>) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("tmp");
        match std::fs::symlink_metadata(&tmp) {
            Ok(m) if m.file_type().is_symlink() => return Err(anyhow!("refusing to write secrets: {} is a symlink", tmp.display())),
            Ok(_) => std::fs::remove_file(&tmp)?,
            Err(_) => {}
        }
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp)?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec_pretty(map)?)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn reload(&self) -> Result<HashMap<String, Credential>> {
        load(&self.path)
    }
}

/// Reads the store. A missing file is empty; an unreadable one is an error,
/// so the next write can't replace every credential with just one.
fn load(path: &std::path::Path) -> Result<HashMap<String, Credential>> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| anyhow!("secrets file {} is damaged ({e}); fix or remove it", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(anyhow!("can't read secrets file {}: {e}", path.display())),
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        // Another process (e.g. the CLI) may have written since we loaded.
        let mut c = self.cache.lock().unwrap();
        if !c.contains_key(key) {
            *c = self.reload()?;
        }
        Ok(c.get(key).cloned())
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        let mut c = self.cache.lock().unwrap();
        *c = self.reload()?;
        c.insert(key.to_string(), value.clone());
        self.flush(&c)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut c = self.cache.lock().unwrap();
        *c = self.reload()?;
        c.remove(key);
        self.flush(&c)
    }
}

#[derive(Default)]
pub struct MemSecretStore {
    map: Mutex<HashMap<String, Credential>>,
}

impl SecretStore for MemSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        self.map.lock().unwrap().insert(key.into(), value.clone());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(p: &str) -> Credential {
        Credential::Password { password: p.into() }
    }

    #[test]
    fn file_store_round_trips_privately() {
        let dir = std::env::temp_dir().join(format!("satchel-secrets-{}", crate::random_token(8)));
        let path = dir.join("secrets.json");
        let s = FileSecretStore::new(path.clone()).unwrap();
        s.set("a", &pw("one")).unwrap();
        // A stale temp file from a crash doesn't get in the way.
        std::fs::write(path.with_extension("tmp"), "junk").unwrap();
        s.set("b", &pw("two")).unwrap();
        assert!(FileSecretStore::new(path.clone()).unwrap().get("a").unwrap().is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn damaged_file_is_an_error_not_empty() {
        let dir = std::env::temp_dir().join(format!("satchel-secrets-{}", crate::random_token(8)));
        let path = dir.join("secrets.json");
        let s = FileSecretStore::new(path.clone()).unwrap();
        s.set("a", &pw("one")).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(s.set("b", &pw("two")).is_err());
        assert!(s.get("c").is_err());
        // The damaged file is left alone for the user to fix.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        assert!(FileSecretStore::new(path).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
