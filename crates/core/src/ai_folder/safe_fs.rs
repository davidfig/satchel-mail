//! Reading and writing in the AI folder without following symlinks.

use super::*;

// Agents can write anywhere in the AI folder, so a symlink they plant must
// never redirect what Satchel reads, writes or deletes there to a file
// outside it (an SSH key copied into `processed/`, a launch agent written
// through a dangling link, a vault emptied through a linked inbox). Every
// read, write and removal of agent-reachable paths goes through these.
//
// On unix each one opens the root and then walks down one directory at a
// time with `openat(O_NOFOLLOW)`, and acts on the last name relative to the
// directory it opened, so a link swapped in between a check and the use
// can't redirect it. A hard link (`nlink > 1`) is refused for reading too:
// it could be another name for a file the agent can't read itself.
//
// Windows has no `openat`; `safe_fs_windows.rs` gets the same guarantees by
// holding each directory open so it can't be swapped, and by checking every
// handle's final path.

/// The most read from one file an agent wrote (a proposal or person file).
pub(crate) const MAX_AGENT_FILE: u64 = 1 << 20;

/// Whether raw messages (`.eml`) and attachments are exported. They're safe
/// to write on every platform now that Windows works from held handles too
/// (a sender's attachment, under a name it chose, must never land outside the
/// folder, such as in a startup folder).
pub(crate) const RAW_FILES_EXPORTED: bool = true;

pub(crate) fn no_symlinks() -> std::io::Error {
    std::io::Error::other("symlinks aren't followed in the AI folder")
}

fn too_large(max: u64) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("file is larger than {max} bytes"))
}

/// Read a regular file under `root` (at most [`MAX_AGENT_FILE`]), refusing
/// symlinks and hard links.
pub(crate) fn read_regular(root: &Path, path: &Path) -> std::io::Result<String> {
    String::from_utf8(read_regular_bytes(root, path)?).map_err(std::io::Error::other)
}

pub(crate) fn read_regular_bytes(root: &Path, path: &Path) -> std::io::Result<Vec<u8>> {
    read_capped(root, path, MAX_AGENT_FILE)
}

/// Write a file only if its content changed (avoids churn for file watchers).
pub(crate) fn write_if_changed(root: &Path, p: &Path, content: &[u8]) -> Result<()> {
    if read_capped(root, p, content.len() as u64).is_ok_and(|old| old == content) {
        return Ok(());
    }
    write_atomic(root, p, content)
}

/// The names from `root` down to `path`; only plain names, no `..`.
fn rel_names(root: &Path, path: &Path) -> std::io::Result<Vec<std::ffi::OsString>> {
    let rest = path.strip_prefix(root).map_err(|_| std::io::Error::other("path is outside the AI folder"))?;
    rest.components()
        .map(|c| match c {
            std::path::Component::Normal(n) => Ok(n.to_os_string()),
            _ => Err(std::io::Error::other("unexpected path component in the AI folder")),
        })
        .collect()
}

#[cfg(unix)]
mod sys {
    use super::{no_symlinks, rel_names, too_large};
    use std::ffi::{CStr, CString, OsStr};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// How deep `clear` descends (and so how many directories it holds open)
    /// before moving what's deeper up to the top.
    const MAX_DEPTH: usize = 32;

    /// How many passes `clear` makes. Each one brings the deepest directories
    /// `MAX_DEPTH` levels up, so this allows trees about 300,000 levels deep.
    const MAX_PASSES: usize = 10_000;

    fn cstr(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    }

    fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
    }

    /// ELOOP and ENOTDIR from an `O_NOFOLLOW` open: a symlink was there.
    fn link_err(e: io::Error) -> io::Error {
        match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => no_symlinks(),
            _ => e,
        }
    }

    fn open_root(root: &Path) -> io::Result<OwnedFd> {
        let c = cstr(root.as_os_str())?;
        let fd = cvt(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) })?;
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn open_sub(dir: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) }).map_err(link_err)?;
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// `lstat` of a name in `dir`; None if there's nothing there.
    fn stat_at(dir: &OwnedFd, name: &CStr) -> io::Result<Option<libc::mode_t>> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        match cvt(unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) }) {
            Ok(_) => Ok(Some(st.st_mode & libc::S_IFMT)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn mkdir_at(dir: &OwnedFd, name: &CStr) -> io::Result<()> {
        match cvt(unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o777) }) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => Err(e),
            _ => Ok(()),
        }
    }

    fn unlink_at(dir: &OwnedFd, name: &CStr, flags: libc::c_int) -> io::Result<()> {
        cvt(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), flags) }).map(drop)
    }

    /// Open the directory `names` leads to from `root`, never through a
    /// symlink. `create` makes missing directories; `replace_links` swaps a
    /// symlink for a real directory instead of refusing it.
    fn walk(root: &Path, names: &[std::ffi::OsString], create: bool, replace_links: bool) -> io::Result<OwnedFd> {
        let mut fd = open_root(root)?;
        for n in names {
            let c = cstr(n)?;
            if replace_links && stat_at(&fd, &c)? == Some(libc::S_IFLNK) {
                tracing::warn!("AI folder: replacing the symlink at {:?} with a directory", n);
                unlink_at(&fd, &c, 0)?;
            }
            fd = match open_sub(&fd, &c) {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    mkdir_at(&fd, &c)?;
                    open_sub(&fd, &c)?
                }
                r => r?,
            };
        }
        Ok(fd)
    }

    /// The directory holding `path`, opened, and the file's name in it.
    fn parent(root: &Path, path: &Path, create: bool) -> io::Result<(OwnedFd, CString)> {
        let mut names = rel_names(root, path)?;
        let last = names.pop().ok_or_else(|| io::Error::other("no file name"))?;
        Ok((walk(root, &names, create, false)?, cstr(&last)?))
    }

    pub fn read_capped(root: &Path, path: &Path, max: u64) -> io::Result<Vec<u8>> {
        let (dir, name) = parent(root, path, false)?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) }).map_err(link_err)?;
        let f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let meta = f.metadata()?;
        if !meta.is_file() {
            return Err(no_symlinks());
        }
        if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
            return Err(io::Error::other("hard-linked files aren't read in the AI folder"));
        }
        if meta.len() > max {
            return Err(too_large(max));
        }
        let mut buf = vec![];
        f.take(max + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > max {
            return Err(too_large(max));
        }
        Ok(buf)
    }

    pub fn write_atomic(root: &Path, path: &Path, content: &[u8]) -> io::Result<()> {
        let (dir, name) = parent(root, path, true)?;
        let tmp = cstr(OsStr::new(&format!(".{}.{}.tmp~", String::from_utf8_lossy(name.as_bytes()), crate::random_token(6))))?;
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let fd = cvt(unsafe { libc::openat(dir.as_raw_fd(), tmp.as_ptr(), flags, 0o666 as libc::c_uint) })?;
        let mut f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let r = f.write_all(content).and_then(|()| {
            // Renaming over the name replaces a symlink there rather than following it.
            cvt(unsafe { libc::renameat(dir.as_raw_fd(), tmp.as_ptr(), dir.as_raw_fd(), name.as_ptr()) }).map(drop)
        });
        if r.is_err() {
            let _ = unlink_at(&dir, &tmp, 0);
        }
        r
    }

    pub fn remove_file(root: &Path, path: &Path) -> io::Result<()> {
        let (dir, name) = parent(root, path, false)?;
        unlink_at(&dir, &name, 0)
    }

    pub fn remove_dir_all(root: &Path, path: &Path) -> io::Result<()> {
        let (dir, name) = match parent(root, path, false) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            r => r?,
        };
        match stat_at(&dir, &name)? {
            None => Ok(()),
            Some(libc::S_IFDIR) => {
                clear(&open_sub(&dir, &name)?)?;
                unlink_at(&dir, &name, libc::AT_REMOVEDIR)
            }
            Some(_) => Err(no_symlinks()),
        }
    }

    /// Remove everything in `top`. Symlinks inside are removed, not followed.
    ///
    /// An agent can nest directories as deep as it likes, so a plain walk
    /// would run out of file handles or stack, and a depth limit would let it
    /// keep files from being removed. Instead each pass goes `MAX_DEPTH`
    /// levels down and moves anything deeper up into `top` (by handle, never
    /// through `..`), and passes repeat until nothing was moved.
    fn clear(top: &OwnedFd) -> io::Result<()> {
        for _ in 0..MAX_PASSES {
            if !clear_pass(top, top, 0)? {
                return Ok(());
            }
        }
        Err(io::Error::other("directories nested too deeply in the AI folder"))
    }

    /// One pass of `clear` over `dir`, `depth` levels below `top`. Returns
    /// whether it moved a directory up to `top`, which needs another pass.
    fn clear_pass(top: &OwnedFd, dir: &OwnedFd, depth: usize) -> io::Result<bool> {
        let mut moved = false;
        for name in names(dir)? {
            if stat_at(dir, &name)? != Some(libc::S_IFDIR) {
                unlink_at(dir, &name, 0)?;
            } else if depth < MAX_DEPTH {
                // A directory that had something moved out of it is left
                // for the next pass, which finds it empty.
                if clear_pass(top, &open_sub(dir, &name)?, depth + 1)? {
                    moved = true;
                } else {
                    unlink_at(dir, &name, libc::AT_REMOVEDIR)?;
                }
            } else {
                let to = cstr(OsStr::new(&format!(".satchel-rm-{}", crate::random_token(12))))?;
                cvt(unsafe { libc::renameat(dir.as_raw_fd(), name.as_ptr(), top.as_raw_fd(), to.as_ptr()) })?;
                moved = true;
            }
        }
        Ok(moved)
    }

    /// The names in an open directory.
    fn names(dir: &OwnedFd) -> io::Result<Vec<CString>> {
        let dup = cvt(unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
        let d = unsafe { libc::fdopendir(dup) };
        if d.is_null() {
            let e = io::Error::last_os_error();
            unsafe { libc::close(dup) };
            return Err(e);
        }
        unsafe { libc::rewinddir(d) };
        let mut out = vec![];
        loop {
            let ent = unsafe { libc::readdir(d) };
            if ent.is_null() {
                break;
            }
            let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
            if name.to_bytes() != b"." && name.to_bytes() != b".." {
                out.push(name.to_owned());
            }
        }
        unsafe { libc::closedir(d) };
        Ok(out)
    }

    pub fn list_files(root: &Path, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let fd = walk(root, &rel_names(root, dir)?, false, false)?;
        let mut out = vec![];
        for name in names(&fd)? {
            if stat_at(&fd, &name)? == Some(libc::S_IFREG) {
                out.push(dir.join(OsStr::from_bytes(name.as_bytes())));
            }
        }
        Ok(out)
    }

    pub fn ensure_real_dir(root: &Path, dir: &Path) -> io::Result<()> {
        walk(root, &rel_names(root, dir)?, true, true).map(drop)
    }
}

/// On Windows, the same guarantees from handles: each directory is held open
/// without share-delete so it can't be swapped, and every handle's final path
/// is checked to be where it should be.
#[cfg(windows)]
#[path = "safe_fs_windows.rs"]
mod sys;

/// Read a regular file under `root`, at most `max` bytes (more is an error).
pub(crate) fn read_capped(root: &Path, path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    sys::read_capped(root, path, max)
}

/// Replace the file at `path` under `root` with `content`: written to a new
/// temp file, then renamed over it, which replaces a symlink there rather
/// than following it. Missing directories are made; symlinked ones refused.
pub(crate) fn write_atomic(root: &Path, path: &Path, content: &[u8]) -> Result<()> {
    Ok(sys::write_atomic(root, path, content)?)
}

/// Remove the file (or symlink, which isn't followed) at `path` under `root`.
/// Refused if a directory on the way is a symlink.
pub(crate) fn remove_file(root: &Path, path: &Path) -> std::io::Result<()> {
    sys::remove_file(root, path)
}

/// Remove the directory at `path` under `root` and everything in it. Refused
/// if it or a directory on the way is a symlink; nothing there is fine.
pub(crate) fn remove_dir_all(root: &Path, path: &Path) -> std::io::Result<()> {
    sys::remove_dir_all(root, path)
}

/// The regular files directly in `dir` under `root` (no symlinks).
pub(crate) fn list_files(root: &Path, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    sys::list_files(root, dir)
}

/// Make `dir` under `root` a real directory, replacing symlinks on the way
/// (removing a link doesn't touch what it points to).
pub(crate) fn ensure_real_dir(root: &Path, dir: &Path) -> std::io::Result<()> {
    sys::ensure_real_dir(root, dir)
}
