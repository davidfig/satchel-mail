//! Content-addressed storage for raw RFC 822 messages.

use std::path::PathBuf;

use anyhow::Result;

pub trait BlobStore: Send + Sync {
    /// Store bytes, returning their BLAKE3 hash.
    fn put(&self, data: &[u8]) -> Result<String>;
    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&self, hash: &str) -> Result<()>;
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
}
