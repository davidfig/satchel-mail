//! Content-addressed storage for raw RFC 822 messages.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

pub trait BlobStore: Send + Sync {
    /// Store bytes, returning their BLAKE3 hash.
    fn put(&self, data: &[u8]) -> Result<String>;
    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&self, hash: &str) -> Result<()>;
    /// Hashes of the stored blobs last written at least `min_age` ago.
    fn list(&self, min_age: Duration) -> Result<Vec<String>>;
}

pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self, hash: &str) -> PathBuf {
        let (a, b) = hash.split_at(2.min(hash.len()));
        self.root.join(a).join(format!("{b}.eml"))
    }
}

impl BlobStore for FsBlobStore {
    fn put(&self, data: &[u8]) -> Result<String> {
        let hash = blake3::hash(data).to_hex().to_string();
        let p = self.path(&hash);
        if !p.exists() {
            std::fs::create_dir_all(p.parent().unwrap())?;
            let tmp = p.with_extension("tmp");
            std::fs::write(&tmp, data)?;
            std::fs::rename(tmp, &p)?;
        }
        Ok(hash)
    }

    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.path(hash)) {
            Ok(d) => Ok(Some(d)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, hash: &str) -> Result<()> {
        let _ = std::fs::remove_file(self.path(hash));
        Ok(())
    }

    fn list(&self, min_age: Duration) -> Result<Vec<String>> {
        let mut out = vec![];
        for dir in std::fs::read_dir(&self.root)? {
            let dir = dir?;
            let prefix = dir.file_name().to_string_lossy().into_owned();
            if !dir.file_type()?.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(dir.path())? {
                let f = f?;
                let name = f.file_name().to_string_lossy().into_owned();
                let Some(rest) = name.strip_suffix(".eml") else { continue };
                let meta = f.metadata()?;
                if meta.is_file() && meta.modified()?.elapsed().unwrap_or_default() >= min_age {
                    out.push(format!("{prefix}{rest}"));
                }
            }
        }
        Ok(out)
    }
}

/// In-memory blobs for tests.
#[derive(Default)]
pub struct MemBlobStore {
    map: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl BlobStore for MemBlobStore {
    fn put(&self, data: &[u8]) -> Result<String> {
        let hash = blake3::hash(data).to_hex().to_string();
        self.map.lock().unwrap().insert(hash.clone(), data.to_vec());
        Ok(hash)
    }
    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.map.lock().unwrap().get(hash).cloned())
    }
    fn delete(&self, hash: &str) -> Result<()> {
        self.map.lock().unwrap().remove(hash);
        Ok(())
    }
    fn list(&self, _min_age: Duration) -> Result<Vec<String>> {
        Ok(self.map.lock().unwrap().keys().cloned().collect())
    }
}
