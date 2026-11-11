//! The sync folder on disk: a header naming the key, and one sealed file
//! per computer that only that computer writes (0080).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::crypto::{self, Key};
use super::state::Entry;

pub const HEADER: &str = "satchel-sync.json";
const README: &str = "README.md";

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    version: u32,
    key_check: String,
}

/// A computer's file as stored: everything but the id is sealed.
#[derive(Serialize, Deserialize)]
struct Sealed {
    format: String,
    version: u32,
    device: String,
    nonce: String,
    data: String,
}

/// A computer's file, opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub device: String,
    pub name: String,
    pub written_at: i64,
    pub items: BTreeMap<String, Entry>,
}

/// The folder's key check, if it holds a sync.
pub fn read_header(dir: &Path) -> Result<Option<String>> {
    let p = dir.join(HEADER);
    if !p.exists() {
        return Ok(None);
    }
    let h: Header = serde_json::from_slice(&std::fs::read(&p)?).with_context(|| format!("{} isn't a sync header", p.display()))?;
    if h.format != "satchel-sync" || h.version != 1 {
        bail!("this folder holds a sync from a newer Satchel");
    }
    Ok(Some(h.key_check))
}

pub fn write_header(dir: &Path, key: &Key) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let h = Header { format: "satchel-sync".into(), version: 1, key_check: crypto::key_check(key) };
    write_atomic(&dir.join(HEADER), &serde_json::to_vec_pretty(&h)?)?;
    write_atomic(
        &dir.join(README),
        b"# Satchel sync\n\nSatchel Mail keeps its settings in step between your computers through this folder: \
          one encrypted file per computer. Mail, passwords and tokens are never here.\n\n\
          Don't edit these files. To stop syncing a computer, turn sync off in Satchel's settings.\n",
    )
}

fn device_path(dir: &Path, device: &str) -> PathBuf {
    dir.join(format!("{device}.json"))
}

pub fn write_snapshot(dir: &Path, key: &Key, snap: &Snapshot) -> Result<()> {
    let (nonce, data) = crypto::seal(key, &snap.device, &serde_json::to_vec(snap)?)?;
    let s = Sealed { format: "satchel-sync-device".into(), version: 1, device: snap.device.clone(), nonce, data };
    write_atomic(&device_path(dir, &snap.device), &serde_json::to_vec(&s)?)
}

pub fn remove_snapshot(dir: &Path, device: &str) -> Result<()> {
    match std::fs::remove_file(device_path(dir, device)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Every other computer's snapshot. A file that can't be read is reported and
/// skipped, so one bad file doesn't stop the rest.
pub fn read_others(dir: &Path, key: &Key, me: &str) -> Result<(Vec<Snapshot>, Vec<String>)> {
    let (mut snaps, mut problems) = (vec![], vec![]);
    for ent in std::fs::read_dir(dir).with_context(|| format!("can't read {}", dir.display()))? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().to_string();
        // iCloud Drive with "Optimize Mac Storage" can move a file off the
        // disk and leave `.name.icloud` in its place: ask for it back, and
        // read it on a later pass.
        if let Some(real) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".icloud")).filter(|n| n.ends_with(".json")) {
            download_from_icloud(&dir.join(real));
            continue;
        }
        let Some(stem) = name.strip_suffix(".json") else { continue };
        if name.starts_with('.') || name == HEADER || stem == me || !ent.file_type()?.is_file() {
            continue;
        }
        match read_one(&ent.path(), key, stem) {
            Ok(Some(s)) => snaps.push(s),
            Ok(None) => {}
            Err(e) => problems.push(format!("{name}: {e:#}")),
        }
    }
    Ok((snaps, problems))
}

fn read_one(path: &Path, key: &Key, stem: &str) -> Result<Option<Snapshot>> {
    let Ok(s) = serde_json::from_slice::<Sealed>(&std::fs::read(path)?) else {
        return Ok(None); // not ours (a folder tool's own file)
    };
    if s.format != "satchel-sync-device" {
        return Ok(None);
    }
    if s.version != 1 {
        bail!("written by a newer Satchel");
    }
    // The id is the associated data, so a file copied under another name fails here.
    if s.device != stem {
        bail!("its name doesn't match the computer that wrote it");
    }
    let snap: Snapshot = serde_json::from_slice(&crypto::open(key, &s.device, &s.nonce, &s.data)?)?;
    if snap.device != s.device {
        bail!("its name doesn't match the computer that wrote it");
    }
    Ok(Some(snap))
}

#[cfg(target_os = "macos")]
fn download_from_icloud(path: &Path) {
    let _ = std::process::Command::new("/usr/bin/brctl").arg("download").arg(path).output();
}

#[cfg(not(target_os = "macos"))]
fn download_from_icloud(_path: &Path) {}

/// A dot-named temporary file, then a rename: folder tools skip dot-names, so
/// they never carry half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.tmp", path.file_name().unwrap().to_string_lossy()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
