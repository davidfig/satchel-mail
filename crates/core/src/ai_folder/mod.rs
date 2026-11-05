//! The AI folder (`~/Satchel/`): how agents work with Satchel through files.
//!
//! `export` writes it, one way: the core never reads it back as truth. Two
//! places are read as proposals instead (`import`): files in
//! `proposals/inbox/`, and person files an agent adds or edits in
//! `contacts/people/` (0029). `safe_fs` keeps both from following symlinks
//! an agent plants.

pub mod export;
pub mod import;
mod readme;
mod relocate;
mod safe_fs;

pub use export::Mirror;
pub(crate) use export::contact_file;
pub use readme::README;
pub(crate) use relocate::relocate;
pub(crate) use safe_fs::*;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;

use crate::blobs::BlobStore;
use crate::store::Store;
pub fn default_root() -> PathBuf {
    directories::UserDirs::new()
        .map(|u| u.home_dir().join("Satchel"))
        .unwrap_or_else(|| PathBuf::from("Satchel"))
}

/// Where the AI folder is: the user's choice (`~` means their home folder),
/// or `~/Satchel`.
pub fn root(settings: &MirrorSettings) -> PathBuf {
    let Some(r) = settings.root.as_deref().map(str::trim).filter(|r| !r.is_empty()) else { return default_root() };
    let home = || directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    match r.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match home() {
            Some(h) => h.join(rest.trim_start_matches('/')),
            None => PathBuf::from(r),
        },
        _ => PathBuf::from(r),
    }
}
