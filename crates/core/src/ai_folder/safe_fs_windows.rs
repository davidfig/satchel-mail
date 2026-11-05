//! The Windows side of `safe_fs`: the same guarantees as the unix `openat`
//! walk, built from handles.
//!
//! Win32 has no `openat`, so each step opens a path, but a path that can't
//! change under it:
//!
//! - Every directory from the root down is opened with
//!   `FILE_FLAG_OPEN_REPARSE_POINT` (so a junction or symlink is opened as
//!   itself, never followed), checked to be a plain directory, and held open
//!   with share mode `READ | WRITE` and no `DELETE`. While it's held nobody
//!   can rename, delete or replace it, so the path of the next step still
//!   leads through exactly these directories.
//! - Paths are built from the root handle's final path (`\\?\C:\…`), so
//!   nothing above the root is resolved again either.
//! - Every handle's final path (`GetFinalPathNameByHandleW`) must be its
//!   parent's final path plus the expected name. That catches the one swap
//!   still possible: an empty directory turned into a junction in place
//!   while held. Anything created through such a junction (an empty
//!   directory, or an empty temp file with a random name) is deleted by
//!   handle and the operation refused.
//! - Files are read, renamed and deleted through the handle that was
//!   checked, never by path again, and a file with more than one link is
//!   never read.

use super::{no_symlinks, too_large};
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO,
    FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE,
    FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileDispositionInfo, FileDispositionInfoEx, FileRenameInfo, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, SYNCHRONIZE, SetFileInformationByHandle, VOLUME_NAME_DOS,
};

/// How deep `clear` descends before moving what's deeper up to the top.
const MAX_DEPTH: usize = 32;

/// How many passes `clear` makes (see the unix side).
const MAX_PASSES: usize = 10_000;

/// Access for a held directory: list it and read its attributes. No DELETE,
/// so our handle never blocks a rename into it.
const DIR_ACCESS: u32 = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE;

/// Share mode for a held directory: no DELETE, so it can't be renamed,
/// deleted or replaced by a junction while we hold it.
const DIR_SHARE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

/// Open the last component itself, not what a link there points to; open
/// directories too.
const NOFOLLOW: u32 = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;

/// An open, checked directory and its final path.
struct Dir {
    _handle: File,
    path: PathBuf,
}

fn cvt(ok: i32) -> io::Result<()> {
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn info(f: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut i = BY_HANDLE_FILE_INFORMATION::default();
    cvt(unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut i) })?;
    Ok(i)
}

/// The handle's final path, `\\?\C:\…` style, as UTF-16.
fn final_path(f: &File) -> io::Result<Vec<u16>> {
    let mut buf = vec![0u16; 512];
    loop {
        let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
        let n = unsafe { GetFinalPathNameByHandleW(f.as_raw_handle(), buf.as_mut_ptr(), buf.len() as u32, flags) } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        // Too small: n is the size needed, counting the terminator.
        if n < buf.len() {
            buf.truncate(n);
            return Ok(buf);
        }
        buf.resize(n, 0);
    }
}

fn lower(s: &[u16]) -> String {
    String::from_utf16_lossy(s).to_lowercase()
}

/// Refuse `f` unless its final path is exactly `parent` plus `name` (the
/// name compared without case, as NTFS does).
fn check_under(f: &File, parent: &Path, name: &OsStr) -> io::Result<()> {
    let got = final_path(f)?;
    let pre: Vec<u16> = parent.as_os_str().encode_wide().collect();
    let sep = u16::from(b'\\');
    let Some(mut rest) = got.strip_prefix(pre.as_slice()) else { return Err(no_symlinks()) };
    if pre.last() != Some(&sep) {
        rest = rest.strip_prefix(&[sep]).ok_or_else(no_symlinks)?;
    }
    let want: Vec<u16> = name.encode_wide().collect();
    if rest.contains(&sep) || lower(rest) != lower(&want) {
        return Err(no_symlinks());
    }
    Ok(())
}

fn is_dir(i: &BY_HANDLE_FILE_INFORMATION) -> bool {
    i.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

fn is_link(i: &BY_HANDLE_FILE_INFORMATION) -> bool {
    i.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn open(path: &Path, access: u32, share: u32) -> io::Result<File> {
    OpenOptions::new().access_mode(access).share_mode(share).custom_flags(NOFOLLOW).open(path)
}

/// Delete the file, empty directory, or link (never its target) `f` is
/// open on. `f` needs DELETE access.
fn delete_by_handle(f: &File) -> io::Result<()> {
    let h = f.as_raw_handle();
    let ex = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    let size = std::mem::size_of_val(&ex) as u32;
    match cvt(unsafe { SetFileInformationByHandle(h, FileDispositionInfoEx, (&raw const ex).cast(), size) }) {
        // Before Windows 10 1709, or a file system without POSIX deletes.
        Err(e) if [ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_INVALID_FUNCTION].map(|c| c as i32).contains(&e.raw_os_error().unwrap_or(0)) => {
            let old = FILE_DISPOSITION_INFO { DeleteFile: true };
            let size = std::mem::size_of_val(&old) as u32;
            cvt(unsafe { SetFileInformationByHandle(h, FileDispositionInfo, (&raw const old).cast(), size) })
        }
        r => r,
    }
}

/// Rename the file `f` is open on (it needs DELETE access) to `to`: a bare
/// name renames it in its own directory; a full path moves it there.
fn rename_by_handle(f: &File, to: &OsStr, replace: bool) -> io::Result<()> {
    let name: Vec<u16> = to.encode_wide().collect();
    let off = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = (off + (name.len() + 1) * 2).max(std::mem::size_of::<FILE_RENAME_INFO>());
    // u64s keep the buffer aligned for the struct's handle field.
    let mut buf = vec![0u64; size.div_ceil(8)];
    let p = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*p).Anonymous.ReplaceIfExists = replace;
        (*p).RootDirectory = std::ptr::null_mut();
        (*p).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), buf.as_mut_ptr().cast::<u8>().add(off).cast::<u16>(), name.len());
        cvt(SetFileInformationByHandle(f.as_raw_handle(), FileRenameInfo, buf.as_ptr().cast(), size as u32))
    }
}

/// Open and hold the root (the user chose it, so a link there is followed).
fn open_root(root: &Path) -> io::Result<Dir> {
    let f = OpenOptions::new().access_mode(DIR_ACCESS).share_mode(DIR_SHARE).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(root)?;
    if !is_dir(&info(&f)?) {
        return Err(io::Error::other("the AI folder isn't a directory"));
    }
    let path = PathBuf::from(OsString::from_wide(&final_path(&f)?));
    Ok(Dir { _handle: f, path })
}

/// Look at `name` in `dir` without following it; None if nothing's there.
fn attrs_at(dir: &Dir, name: &OsStr) -> io::Result<Option<BY_HANDLE_FILE_INFORMATION>> {
    let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    match open(&dir.path.join(name), FILE_READ_ATTRIBUTES | SYNCHRONIZE, all) {
        Ok(f) => {
            check_under(&f, &dir.path, name)?;
            Ok(Some(info(&f)?))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Delete `name` in `dir` by handle, never following it. A plain directory
/// is only removed if `dirs` (and it's empty).
fn delete_at(dir: &Dir, name: &OsStr, dirs: bool) -> io::Result<()> {
    let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let f = open(&dir.path.join(name), DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE, all)?;
    check_under(&f, &dir.path, name)?;
    let i = info(&f)?;
    if is_dir(&i) && !is_link(&i) && !dirs {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, "is a directory"));
    }
    delete_by_handle(&f)
}

/// Open and hold the plain directory `name` in `dir`. `create` makes it if
/// it's missing.
fn open_sub(dir: &Dir, name: &OsStr, create: bool) -> io::Result<Dir> {
    let path = dir.path.join(name);
    let mut created = false;
    let f = match open(&path, DIR_ACCESS, DIR_SHARE) {
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
            match std::fs::create_dir(&path) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
                r => created = r.is_ok(),
            }
            open(&path, DIR_ACCESS, DIR_SHARE)?
        }
        r => r?,
    };
    if let Err(e) = check_under(&f, &dir.path, name) {
        // Made through a junction swapped in above: take back what we made.
        drop(f);
        if created && let Ok(f) = open(&path, DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE, DIR_SHARE | FILE_SHARE_DELETE) {
            let _ = delete_by_handle(&f);
        }
        return Err(e);
    }
    let i = info(&f)?;
    if is_link(&i) || !is_dir(&i) {
        return Err(no_symlinks());
    }
    Ok(Dir { _handle: f, path })
}

/// Open and hold every directory `names` leads through from `root`; the
/// last is the one wanted. `replace_links` swaps a link for a real
/// directory instead of refusing it.
fn walk(root: &Path, names: &[OsString], create: bool, replace_links: bool) -> io::Result<Vec<Dir>> {
    let mut chain = vec![open_root(root)?];
    for n in names {
        let dir = chain.last().expect("root");
        if replace_links && attrs_at(dir, n)?.is_some_and(|i| is_link(&i)) {
            tracing::warn!("AI folder: replacing the link at {:?} with a directory", n);
            delete_at(dir, n, false)?;
        }
        let sub = open_sub(dir, n, create)?;
        chain.push(sub);
    }
    Ok(chain)
}

/// The held directories down to the one holding `path`, and the file's name.
fn parent(root: &Path, path: &Path, create: bool) -> io::Result<(Vec<Dir>, OsString)> {
    let mut names = super::rel_names(root, path)?;
    let last = names.pop().ok_or_else(|| io::Error::other("no file name"))?;
    Ok((walk(root, &names, create, false)?, last))
}

pub fn read_capped(root: &Path, path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let (chain, name) = parent(root, path, false)?;
    let dir = chain.last().expect("root");
    let f = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).custom_flags(NOFOLLOW).open(dir.path.join(&name))?;
    let i = info(&f)?;
    if is_link(&i) || is_dir(&i) {
        return Err(no_symlinks());
    }
    if i.nNumberOfLinks > 1 {
        return Err(io::Error::other("hard-linked files aren't read in the AI folder"));
    }
    check_under(&f, &dir.path, &name)?;
    if (u64::from(i.nFileSizeHigh) << 32 | u64::from(i.nFileSizeLow)) > max {
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
    let (chain, name) = parent(root, path, true)?;
    let dir = chain.last().expect("root");
    let tmp = OsString::from(format!(".{}.{}.tmp~", name.to_string_lossy(), crate::random_token(6)));
    // create_new also makes std add FILE_FLAG_OPEN_REPARSE_POINT.
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES | DELETE)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(NOFOLLOW)
        .open(dir.path.join(&tmp))?;
    let r = check_under(&f, &dir.path, &tmp)
        .and_then(|()| f.write_all(content))
        // In place, by handle: the temp file can't be moved meanwhile (no
        // share DELETE), and renaming over a link replaces the link.
        .and_then(|()| rename_by_handle(&f, &name, true))
        .and_then(|()| check_under(&f, &dir.path, &name));
    if r.is_err() {
        let _ = delete_by_handle(&f);
    }
    r
}

pub fn remove_file(root: &Path, path: &Path) -> io::Result<()> {
    let (chain, name) = parent(root, path, false)?;
    delete_at(chain.last().expect("root"), &name, false)
}

pub fn remove_dir_all(root: &Path, path: &Path) -> io::Result<()> {
    let (chain, name) = match parent(root, path, false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let dir = chain.last().expect("root");
    match attrs_at(dir, &name)? {
        None => Ok(()),
        Some(i) if is_dir(&i) && !is_link(&i) => {
            // Held without DELETE while clearing, so things can be moved
            // into it; then reopened to delete it.
            clear(&open_sub(dir, &name, false)?)?;
            delete_at(dir, &name, true)
        }
        Some(_) => Err(no_symlinks()),
    }
}

/// Remove everything in `top`; links inside are removed, not followed. Deep
/// trees are handled in passes, as on unix.
fn clear(top: &Dir) -> io::Result<()> {
    for _ in 0..MAX_PASSES {
        if !clear_pass(top, top, 0)? {
            return Ok(());
        }
    }
    Err(io::Error::other("directories nested too deeply in the AI folder"))
}

fn clear_pass(top: &Dir, dir: &Dir, depth: usize) -> io::Result<bool> {
    let mut moved = false;
    for name in names(dir)? {
        let Some(i) = attrs_at(dir, &name)? else { continue };
        if !is_dir(&i) || is_link(&i) {
            delete_at(dir, &name, false)?;
        } else if depth < MAX_DEPTH {
            let sub = open_sub(dir, &name, false)?;
            let deeper = clear_pass(top, &sub, depth + 1)?;
            drop(sub);
            if deeper {
                moved = true;
            } else {
                delete_at(dir, &name, true)?;
            }
        } else {
            let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
            let f = open(&dir.path.join(&name), DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE, all)?;
            check_under(&f, &dir.path, &name)?;
            let to = top.path.join(format!(".satchel-rm-{}", crate::random_token(12)));
            rename_by_handle(&f, to.as_os_str(), false)?;
            moved = true;
        }
    }
    Ok(moved)
}

/// The names in a held directory.
fn names(dir: &Dir) -> io::Result<Vec<OsString>> {
    std::fs::read_dir(&dir.path)?.map(|e| e.map(|e| e.file_name())).collect()
}

pub fn list_files(root: &Path, dir: &Path) -> io::Result<Vec<PathBuf>> {
    let chain = walk(root, &super::rel_names(root, dir)?, false, false)?;
    let held = chain.last().expect("root");
    let mut out = vec![];
    for name in names(held)? {
        if attrs_at(held, &name)?.is_some_and(|i| !is_dir(&i) && !is_link(&i)) {
            out.push(dir.join(name));
        }
    }
    Ok(out)
}

pub fn ensure_real_dir(root: &Path, dir: &Path) -> io::Result<()> {
    walk(root, &super::rel_names(root, dir)?, true, true).map(drop)
}
