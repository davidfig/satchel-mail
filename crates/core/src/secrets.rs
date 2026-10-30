//! Credential storage. Desktop uses the OS keychain; tests and headless dev
//! use a 0600 JSON file. Secrets never touch the main database.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use satchel_api::Credential;
use serde_json::Value;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Credential>>;
    fn set(&self, key: &str, value: &Credential) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub fn account_key(account_id: &str) -> String {
    format!("account:{account_id}")
}

/// OS keychain store. Every credential lives in one keychain item, so macOS
/// asks the user once per app instead of once per account (0052).
#[cfg(feature = "keychain")]
pub struct KeychainSecretStore(BundleStore<KeychainItems>);

#[cfg(feature = "keychain")]
impl KeychainSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        // The keychain is the user's, whatever data directory a process uses,
        // so the lock is in the user's own one.
        let lock = crate::engine::platform_data_dir().map(|d| d.join("credentials.lock"));
        Self(BundleStore::new(KeychainItems { service: service.into() }).with_lock(lock))
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeychainSecretStore {
    fn get(&self, key: &str) -> Result<Option<Credential>> {
        self.0.get(key)
    }
    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.delete(key)
    }
}

#[cfg(feature = "keychain")]
struct KeychainItems {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeychainItems {
    fn entry(&self, name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, name).map_err(|e| anyhow!("keychain: {e}"))
    }
}

#[cfg(feature = "keychain")]
impl Items for KeychainItems {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match self.entry(name)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow!("keychain: {e}")),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.entry(name)?.set_password(value).map_err(|e| anyhow!("keychain: {e}"))
    }
    fn delete(&self, name: &str) -> Result<()> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow!("keychain: {e}")),
        }
    }
}

/// Named string items in a backing store (the keychain, or a fake in tests).
trait Items: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// The item holding every credential, as a JSON object keyed like the trait.
const BUNDLE: &str = "credentials";

/// Keeps all credentials in the single `BUNDLE` item. Older versions stored
/// one item per key; those move into the bundle the first time they're read.
struct BundleStore<I> {
    items: I,
    /// The bundle as last read or written. A miss reloads it, since another
    /// process (the CLI) may have added the key since.
    cache: Mutex<Option<BTreeMap<String, Value>>>,
    /// Held while the bundle is read and rewritten, so two processes can't
    /// each drop the other's change.
    lock: Option<PathBuf>,
}

impl<I: Items> BundleStore<I> {
    fn new(items: I) -> Self {
        Self { items, cache: Mutex::new(None), lock: None }
    }

    #[cfg_attr(not(feature = "keychain"), allow(dead_code))]
    fn with_lock(mut self, lock: Option<PathBuf>) -> Self {
        self.lock = lock;
        self
    }

    fn load(&self) -> Result<BTreeMap<String, Value>> {
        match self.items.get(BUNDLE)? {
            Some(s) => serde_json::from_str(&s).map_err(|e| anyhow!("keychain item \"{BUNDLE}\" is damaged: {e}")),
            None => Ok(BTreeMap::new()),
        }
    }

    fn get(&self, key: &str) -> Result<Option<Credential>> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(v) = cache.as_ref().and_then(|m| m.get(key)) {
            return Ok(Some(serde_json::from_value(v.clone())?));
        }
        let _held = self.lock.as_deref().and_then(lock_file);
        let mut map = self.load()?;
        if !map.contains_key(key)
            && let Some(old) = self.items.get(key)?
        {
            map.insert(key.to_string(), serde_json::from_str(&old)?);
            self.items.set(BUNDLE, &serde_json::to_string(&map)?)?;
            // Only once the bundle has it; a leftover item is harmless.
            let _ = self.items.delete(key);
        }
        let found = map.get(key).cloned();
        *cache = Some(map);
        Ok(found.map(serde_json::from_value).transpose()?)
    }

    fn set(&self, key: &str, value: &Credential) -> Result<()> {
        let mut cache = self.cache.lock().unwrap();
        let _held = self.lock.as_deref().and_then(lock_file);
        let mut map = self.load()?;
        let value = serde_json::to_value(value)?;
        // Rewriting an unchanged item can still prompt.
        if map.get(key) != Some(&value) {
            map.insert(key.to_string(), value);
            self.items.set(BUNDLE, &serde_json::to_string(&map)?)?;
        }
        *cache = Some(map);
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut cache = self.cache.lock().unwrap();
        let _held = self.lock.as_deref().and_then(lock_file);
        let mut map = self.load()?;
        if map.remove(key).is_some() {
            self.items.set(BUNDLE, &serde_json::to_string(&map)?)?;
        }
        self.items.delete(key)?;
        *cache = Some(map);
        Ok(())
    }
}

/// Takes an exclusive lock on `path` (created if needed), released when the
/// file is dropped. Best effort: if the lock can't be had, the write goes
/// ahead unlocked rather than failing.
fn lock_file(path: &std::path::Path) -> Option<std::fs::File> {
    if let Some(d) = path.parent() {
        let _ = crate::engine::create_private_dir(d);
    }
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path);
    match f.and_then(|f| f.lock().map(|()| f)) {
        Ok(f) => Some(f),
        Err(e) => {
            tracing::warn!("can't lock {}: {e}", path.display());
            None
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
        let _held = lock_file(&self.path.with_extension("lock"));
        *c = self.reload()?;
        c.insert(key.to_string(), value.clone());
        self.flush(&c)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut c = self.cache.lock().unwrap();
        let _held = lock_file(&self.path.with_extension("lock"));
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

    #[derive(Default)]
    struct FakeItems(Mutex<HashMap<String, String>>, Mutex<usize>);

    impl Items for FakeItems {
        fn get(&self, name: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<()> {
            *self.1.lock().unwrap() += 1;
            self.0.lock().unwrap().insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<()> {
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }

    fn password(c: Option<Credential>) -> Option<String> {
        match c? {
            Credential::Password { password } => Some(password),
            Credential::OAuth { .. } => None,
        }
    }

    #[test]
    fn bundle_keeps_every_credential_in_one_item() {
        let s = BundleStore::new(FakeItems::default());
        s.set("account:a", &pw("one")).unwrap();
        s.set("account:b", &pw("two")).unwrap();
        s.set("account:b", &pw("two")).unwrap();
        assert_eq!(*s.items.1.lock().unwrap(), 2, "an unchanged value isn't rewritten");
        assert_eq!(s.items.0.lock().unwrap().keys().collect::<Vec<_>>(), vec![BUNDLE]);
        s.delete("account:a").unwrap();
        assert_eq!(password(s.get("account:a").unwrap()), None);
        assert_eq!(password(s.get("account:b").unwrap()).as_deref(), Some("two"));
    }

    #[test]
    fn bundle_moves_old_per_key_items_in() {
        let items = FakeItems::default();
        items.0.lock().unwrap().insert("account:a".into(), serde_json::to_string(&pw("old")).unwrap());
        let s = BundleStore::new(items);
        s.set("account:b", &pw("new")).unwrap();
        assert_eq!(password(s.get("account:a").unwrap()).as_deref(), Some("old"));
        let map = s.items.0.lock().unwrap();
        assert!(!map.contains_key("account:a"));
        assert!(map[BUNDLE].contains("old") && map[BUNDLE].contains("new"));
    }

    #[test]
    fn bundle_sees_keys_another_process_added() {
        let s = BundleStore::new(FakeItems::default());
        assert!(s.get("account:a").unwrap().is_none());
        let other = BundleStore::new(FakeItems::default());
        other.set("account:a", &pw("cli")).unwrap();
        let written = other.items.0.lock().unwrap()[BUNDLE].clone();
        s.items.0.lock().unwrap().insert(BUNDLE.into(), written);
        assert_eq!(password(s.get("account:a").unwrap()).as_deref(), Some("cli"));
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
