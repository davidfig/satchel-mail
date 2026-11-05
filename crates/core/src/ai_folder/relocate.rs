//! Moving the AI folder when the user picks a new location, so proposals
//! waiting in it and files agents keep there come along.

use std::path::Path;

use anyhow::{Context, Result};

use crate::error::{err, invalid};
use satchel_api::ErrorCode;

/// Move the AI folder from `from` to `to`. Nothing to move if `from` doesn't
/// exist. `to` must be absent or an empty folder, so we never mix mail into
/// someone's existing files.
pub(crate) fn relocate(from: &Path, to: &Path) -> Result<()> {
    if !to.is_absolute() {
        return Err(invalid("Choose a full path for the AI folder, such as ~/Satchel"));
    }
    if to.starts_with(from) || from.starts_with(to) {
        return Err(invalid("The new AI folder can't be inside the old one, or the other way round"));
    }
    match std::fs::symlink_metadata(from) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(super::no_symlinks().into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    match std::fs::read_dir(to) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(err(ErrorCode::Conflict, format!("{} already has files in it. Choose an empty or new folder.", to.display())));
            }
            std::fs::remove_dir(to)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("can't use {}", to.display())),
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("can't create {}", parent.display()))?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Another disk: copy, then remove the old folder.
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            if let Err(e) = copy_tree(from, to, 0) {
                let _ = std::fs::remove_dir_all(to);
                return Err(e).with_context(|| format!("couldn't copy the AI folder to {}", to.display()));
            }
            std::fs::remove_dir_all(from).with_context(|| format!("copied, but couldn't remove {}", from.display()))
        }
        Err(e) => Err(e).with_context(|| format!("couldn't move the AI folder to {}", to.display())),
    }
}

/// Copy a directory tree. Symlinks an agent planted are skipped, not followed.
fn copy_tree(from: &Path, to: &Path, depth: usize) -> std::io::Result<()> {
    if depth > 32 {
        return Err(std::io::Error::other("directories nested too deeply in the AI folder"));
    }
    std::fs::create_dir(to)?;
    #[cfg(unix)]
    std::fs::set_permissions(to, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest, depth + 1)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("satchel-relocate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn moves_into_a_new_folder() {
        let d = tmp("new");
        let (from, to) = (d.join("old"), d.join("a/b/new"));
        std::fs::create_dir_all(from.join("proposals/inbox")).unwrap();
        std::fs::write(from.join("proposals/inbox/x.md"), "hi").unwrap();
        relocate(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(to.join("proposals/inbox/x.md")).unwrap(), "hi");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_a_folder_with_files() {
        let d = tmp("full");
        let (from, to) = (d.join("old"), d.join("docs"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("taxes.pdf"), "x").unwrap();
        assert!(relocate(&from, &to).is_err());
        assert!(from.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_nesting_and_relative_paths() {
        let d = tmp("nest");
        let from = d.join("old");
        std::fs::create_dir_all(&from).unwrap();
        assert!(relocate(&from, &from.join("inner")).is_err());
        assert!(relocate(&from, Path::new("Satchel")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_skips_symlinks() {
        let d = tmp("copy");
        let (from, to) = (d.join("old"), d.join("new"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("a.md"), "a").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", from.join("evil")).unwrap();
        copy_tree(&from, &to, 0).unwrap();
        assert!(to.join("a.md").exists());
        assert!(std::fs::symlink_metadata(to.join("evil")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
