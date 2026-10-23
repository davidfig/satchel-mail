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

#[cfg(feature = "keychain")]
pub struct KeychainSecretStore {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeychainSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }
    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(|e| anyhow!("keychain: {e}"))
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeychainSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        match self.entry(key)?.get_password() {
            Ok(s) => Ok(Some(serde_json::from_str(&s)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow!("keychain: {e}")),
        }
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        self.entry(key)?
            .set_password(&serde_json::to_string(value)?)
            .map_err(|e| anyhow!("keychain: {e}"))
    }
    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow!("keychain: {e}")),
        }
    }
}

/// JSON file store (permissions 0600). Used for tests, CI and headless servers.
pub struct FileSecretStore {
    path: PathBuf,
    cache: Mutex<HashMap<String, Credential>>,
}

impl FileSecretStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        let cache = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        Ok(Self { path, cache: Mutex::new(cache) })
    }

    fn flush(&self, map: &HashMap<String, Credential>) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(map)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn reload(&self) -> HashMap<String, Credential> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        // Another process (e.g. the CLI) may have written since we loaded.
        let mut c = self.cache.lock().unwrap();
        if !c.contains_key(key) {
            *c = self.reload();
        }
        Ok(c.get(key).cloned())
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        let mut c = self.cache.lock().unwrap();
        *c = self.reload();
        c.insert(key.to_string(), value.clone());
        self.flush(&c)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut c = self.cache.lock().unwrap();
        *c = self.reload();
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
