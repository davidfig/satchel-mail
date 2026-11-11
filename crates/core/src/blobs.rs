//! Content-addressed storage for raw RFC 822 messages, gzip-compressed on
//! disk (0074). A blob's hash is always of the uncompressed bytes.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

/// What [`BlobStore::put`] stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// BLAKE3 of the uncompressed bytes.
    pub hash: String,
    /// Bytes the copy takes on disk.
    pub size: u64,
}

pub trait BlobStore: Send + Sync {
    /// Store bytes, returning their hash and the space they take.
    fn put(&self, data: &[u8]) -> Result<Stored>;
    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&self, hash: &str) -> Result<()>;
    /// Hashes of the stored blobs last written at least `min_age` ago.
    fn list(&self, min_age: Duration) -> Result<Vec<String>>;
    /// Compress up to `limit` blobs still stored uncompressed by older
    /// versions. Returns each one's hash and new size; empty when none are
    /// left.
    fn compress_legacy(&self, _limit: usize) -> Result<Vec<Stored>> {
        Ok(vec![])
    }
}

const GZ: &str = "gz";
/// Uncompressed copies written before 0074.
const LEGACY: &str = "eml";

pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self, hash: &str, ext: &str) -> PathBuf {
        let (a, b) = hash.split_at(2.min(hash.len()));
        self.root.join(a).join(format!("{b}.{ext}"))
    }

    /// Write `data` compressed under `hash`, replacing an uncompressed copy.
    fn write_gz(&self, hash: &str, data: &[u8]) -> Result<u64> {
        let p = self.path(hash, GZ);
        std::fs::create_dir_all(p.parent().unwrap())?;
        // Unique, so two writers of the same blob can't interleave in one file.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = p.with_extension(format!("{}-{n}.tmp", std::process::id()));
        let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(data.len() / 3), flate2::Compression::default());
        enc.write_all(data)?;
        let gz = enc.finish()?;
        std::fs::write(&tmp, &gz)?;
        std::fs::rename(tmp, &p)?;
        let _ = std::fs::remove_file(self.path(hash, LEGACY));
        Ok(gz.len() as u64)
    }

    fn read_gz(&self, hash: &str) -> std::io::Result<Vec<u8>> {
        let f = std::fs::File::open(self.path(hash, GZ))?;
        let mut out = vec![];
        flate2::read::GzDecoder::new(std::io::BufReader::new(f)).read_to_end(&mut out)?;
        Ok(out)
    }

    /// Hashes of the stored files with extension `ext`, with their metadata.
    fn files(&self, ext: &str) -> Result<Vec<(String, std::fs::Metadata)>> {
        let suffix = format!(".{ext}");
        let mut out = vec![];
        for dir in std::fs::read_dir(&self.root)? {
            let dir = dir?;
            if !dir.file_type()?.is_dir() {
                continue;
            }
            let prefix = dir.file_name().to_string_lossy().into_owned();
            for f in std::fs::read_dir(dir.path())? {
                let f = f?;
                let name = f.file_name().to_string_lossy().into_owned();
                let Some(rest) = name.strip_suffix(&suffix) else { continue };
                let meta = f.metadata()?;
                if meta.is_file() {
                    out.push((format!("{prefix}{rest}"), meta));
                }
            }
        }
        Ok(out)
    }
}

fn not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

impl BlobStore for FsBlobStore {
    fn put(&self, data: &[u8]) -> Result<Stored> {
        let hash = blake3::hash(data).to_hex().to_string();
        let size = match std::fs::metadata(self.path(&hash, GZ)) {
            Ok(m) => m.len(),
            Err(e) if not_found(&e) => self.write_gz(&hash, data)?,
            Err(e) => return Err(e.into()),
        };
        Ok(Stored { hash, size })
    }

    fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        match self.read_gz(hash) {
            Ok(d) => return Ok(Some(d)),
            Err(e) if !not_found(&e) => return Err(e.into()),
            Err(_) => {}
        }
        match std::fs::read(self.path(hash, LEGACY)) {
            Ok(d) => Ok(Some(d)),
            // Compressed by another process between the two reads.
            Err(e) if not_found(&e) => match self.read_gz(hash) {
                Ok(d) => Ok(Some(d)),
                Err(e) if not_found(&e) => Ok(None),
                Err(e) => Err(e.into()),
            },
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, hash: &str) -> Result<()> {
        let _ = std::fs::remove_file(self.path(hash, GZ));
        let _ = std::fs::remove_file(self.path(hash, LEGACY));
        Ok(())
    }

    fn list(&self, min_age: Duration) -> Result<Vec<String>> {
        let mut out = vec![];
        for ext in [GZ, LEGACY] {
            for (hash, meta) in self.files(ext)? {
                if meta.modified()?.elapsed().unwrap_or_default() >= min_age {
                    out.push(hash);
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn compress_legacy(&self, limit: usize) -> Result<Vec<Stored>> {
        let mut out = vec![];
        for (hash, _) in self.files(LEGACY)? {
            if out.len() >= limit {
                break;
            }
            let data = match std::fs::read(self.path(&hash, LEGACY)) {
                Ok(d) => d,
                Err(e) if not_found(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            // A copy whose bytes don't match its name is left as it is
            // rather than stored under a hash it doesn't have.
            if blake3::hash(&data).to_hex().as_str() != hash {
                tracing::warn!("blobs: {hash} doesn't match its contents; left uncompressed");
                continue;
            }
            let size = self.write_gz(&hash, &data)?;
            out.push(Stored { hash, size });
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
    fn put(&self, data: &[u8]) -> Result<Stored> {
        let hash = blake3::hash(data).to_hex().to_string();
        self.map.lock().unwrap().insert(hash.clone(), data.to_vec());
        Ok(Stored { hash, size: data.len() as u64 })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, FsBlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = FsBlobStore::new(dir.path().join("blobs")).unwrap();
        (dir, s)
    }

    fn message() -> Vec<u8> {
        let mut m = b"From: a@x.test\r\nSubject: hi\r\n\r\n".to_vec();
        for i in 0..200 {
            m.extend_from_slice(format!("<p>Line {i} of a newsletter, much like the others.</p>\r\n").as_bytes());
        }
        m
    }

    #[test]
    fn stores_compressed_and_reads_back() {
        let (_d, s) = store();
        let m = message();
        let put = s.put(&m).unwrap();
        assert!(put.size < m.len() as u64 / 3, "{} of {}", put.size, m.len());
        assert!(s.path(&put.hash, GZ).is_file());
        assert!(!s.path(&put.hash, LEGACY).exists());
        assert_eq!(s.get(&put.hash).unwrap().unwrap(), m);
        // Storing it again changes nothing and reports the same size.
        assert_eq!(s.put(&m).unwrap(), put);
        assert_eq!(s.list(Duration::ZERO).unwrap(), vec![put.hash.clone()]);
        s.delete(&put.hash).unwrap();
        assert_eq!(s.get(&put.hash).unwrap(), None);
    }

    #[test]
    fn reads_and_compresses_uncompressed_copies() {
        let (_d, s) = store();
        let m = message();
        let hash = blake3::hash(&m).to_hex().to_string();
        let legacy = s.path(&hash, LEGACY);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, &m).unwrap();
        // A copy that doesn't match its name is left alone.
        let bad = "ff".repeat(32);
        std::fs::create_dir_all(s.path(&bad, LEGACY).parent().unwrap()).unwrap();
        std::fs::write(s.path(&bad, LEGACY), b"not what the name says").unwrap();

        assert_eq!(s.get(&hash).unwrap().unwrap(), m);
        let mut listed = s.list(Duration::ZERO).unwrap();
        listed.sort();
        assert_eq!(listed.len(), 2);

        let done = s.compress_legacy(10).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].hash, hash);
        assert!(!legacy.exists());
        assert_eq!(s.get(&hash).unwrap().unwrap(), m);
        assert_eq!(done[0].size, std::fs::metadata(s.path(&hash, GZ)).unwrap().len());
        assert!(s.compress_legacy(10).unwrap().is_empty());
        assert!(s.path(&bad, LEGACY).exists());
        s.delete(&bad).unwrap();
    }

    #[test]
    fn putting_over_an_uncompressed_copy_replaces_it() {
        let (_d, s) = store();
        let m = message();
        let hash = blake3::hash(&m).to_hex().to_string();
        std::fs::create_dir_all(s.path(&hash, LEGACY).parent().unwrap()).unwrap();
        std::fs::write(s.path(&hash, LEGACY), &m).unwrap();
        let put = s.put(&m).unwrap();
        assert!(!s.path(&hash, LEGACY).exists());
        assert_eq!(s.get(&put.hash).unwrap().unwrap(), m);
    }
}
