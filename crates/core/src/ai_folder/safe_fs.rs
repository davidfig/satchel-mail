//! Reading and writing in the AI folder without following symlinks.

use super::*;

// Agents can write anywhere in the AI folder, so a symlink they plant must
// never redirect what Satchel reads or writes there to a file outside it
// (an SSH key copied into `processed/`, a launch agent written through a
// dangling link). Every read and write of agent-reachable paths goes through
// these.

/// Whether `path`, or a directory between `root` and it, is a symlink.
/// `root` itself may be one (the user chose it).
pub(crate) fn has_symlink_below(root: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(root) else { return true };
    let mut cur = root.to_path_buf();
    rest.components().any(|c| {
        cur.push(c);
        std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink())
    })
}

pub(crate) fn no_symlinks() -> std::io::Error {
    std::io::Error::other("symlinks aren't followed in the AI folder")
}

/// Read a regular file under `root`, refusing symlinks. The opened file is
/// checked against the path, so a link swapped in meanwhile is caught too.
pub(crate) fn read_regular(root: &Path, path: &Path) -> std::io::Result<String> {
    String::from_utf8(read_regular_bytes(root, path)?).map_err(std::io::Error::other)
}

pub(crate) fn read_regular_bytes(root: &Path, path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    if has_symlink_below(root, path) {
        return Err(no_symlinks());
    }
    let before = std::fs::symlink_metadata(path)?;
    let mut f = std::fs::File::open(path)?;
    let opened = f.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
            return Err(no_symlinks());
        }
    }
    if !before.is_file() || !opened.is_file() {
        return Err(no_symlinks());
    }
    let mut buf = vec![];
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Replace the file at `path` under `root` with `content`: written to a new
/// temp file, then renamed over it, which replaces a symlink there rather
/// than following it.
pub(crate) fn write_atomic(root: &Path, path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| anyhow::anyhow!("no parent directory"))?;
    if has_symlink_below(root, dir) {
        return Err(no_symlinks().into());
    }
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp~", crate::random_token(6)));
    let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).and_then(|mut f| f.write_all(content));
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Write a file only if its content changed (avoids churn for file watchers).
pub(crate) fn write_if_changed(root: &Path, p: &Path, content: &[u8]) -> Result<()> {
    if read_regular_bytes(root, p).is_ok_and(|old| old == content) {
        return Ok(());
    }
    write_atomic(root, p, content)
}
