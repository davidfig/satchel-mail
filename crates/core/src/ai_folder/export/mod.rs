//! Writing the AI folder: one Markdown file per thread, indexes, contacts and the agenda.

mod calendar;
mod contacts;
mod threads;

pub(crate) use contacts::contact_file;

use super::*;

pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "no-subject".into() } else { out }
}

pub(crate) fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || "@._- ".contains(c) { c } else { '_' })
        .collect::<String>()
        .trim()
        .to_string()
}

pub(crate) fn fmt_addrs(v: &[Address]) -> String {
    v.iter().map(|a| a.display()).collect::<Vec<_>>().join(", ")
}

pub(crate) fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} B")
    }
}
pub struct Mirror<'a> {
    pub store: &'a Store,
    pub blobs: &'a dyn BlobStore,
    pub settings: &'a MirrorSettings,
    /// What was last written to each `contacts/people/` file, by file name,
    /// so an agent's edit can be told apart from an out-of-date copy.
    pub contact_files: &'a std::sync::Mutex<HashMap<String, String>>,
}

impl Mirror<'_> {
    fn root(&self) -> PathBuf {
        root(self.settings)
    }

    pub fn ensure_layout(&self) -> Result<()> {
        let r = self.root();
        std::fs::create_dir_all(r.join("proposals/inbox"))?;
        std::fs::create_dir_all(r.join("proposals/processed"))?;
        // Mail is private: other accounts on this computer can't read it.
        #[cfg(unix)]
        std::fs::set_permissions(&r, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        write_if_changed(&self.root(), &r.join("README.md"), README.as_bytes())?;
        Ok(())
    }

    fn account_allowed(&self, account_id: &str) -> bool {
        self.settings.account_ids.as_ref().is_none_or(|a| a.iter().any(|x| x == account_id))
    }
}
