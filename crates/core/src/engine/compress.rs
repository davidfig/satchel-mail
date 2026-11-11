//! Compressing stored copies written uncompressed by older versions (0074).

use super::*;
use std::time::Duration;

use tracing::{debug, warn};

const KEY: &str = "blob_format";
const DONE: &str = "gz-v1";
/// Copies compressed per batch, and the pause between batches so the
/// conversion stays in the background.
const BATCH: usize = 200;
const PAUSE: Duration = Duration::from_millis(100);

impl Engine {
    /// Once per data directory, compresses every uncompressed copy and
    /// records its new size, so storage limits count what's on disk. Picks
    /// up where it stopped on the next start.
    pub(crate) async fn compress_worker(this: std::sync::Weak<Self>) {
        let mut total = 0;
        loop {
            let Some(e) = this.upgrade() else { return };
            let e2 = e.clone();
            let r = tokio::task::spawn_blocking(move || crate::store::background(|| e2.compress_batch(BATCH))).await;
            drop(e);
            match r {
                Ok(Ok(Some(n))) => total += n,
                Ok(Ok(None)) => {
                    if total > 0 {
                        debug!("blobs: compressed {total}");
                    }
                    return;
                }
                Ok(Err(err)) => {
                    warn!("blobs: compressing: {err:#}");
                    return;
                }
                Err(_) => return,
            }
            tokio::time::sleep(PAUSE).await;
        }
    }

    /// Compress up to `n` uncompressed copies. Returns how many, or None
    /// once there are none left (and records that).
    pub fn compress_batch(&self, n: usize) -> Result<Option<usize>> {
        if self.store.get_setting::<String>(KEY)?.as_deref() == Some(DONE) {
            return Ok(None);
        }
        let done = self.blobs.compress_legacy(n)?;
        if done.is_empty() {
            self.store.set_setting(KEY, &DONE)?;
            return Ok(None);
        }
        let sizes: Vec<(String, u64)> = done.into_iter().map(|s| (s.hash, s.size)).collect();
        self.store.set_blob_sizes(&sizes)?;
        Ok(Some(sizes.len()))
    }
}
