//! Writing the AI folder: one Markdown file per thread, indexes, contacts,
//! the agenda and `sync.json`.

mod calendar;
mod contacts;
mod status;
mod threads;
mod visible;

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

/// The longest name `safe_name` makes, in bytes, well under the usual
/// 255-byte limit so a prefix or extension still fits.
const MAX_NAME: usize = 120;

/// A file or directory name from untrusted text: odd characters become `_`,
/// and a long name is cut and ends in a hash of the whole, so it still fits
/// and stays distinct.
pub(crate) fn safe_name(s: &str) -> String {
    let name = s.chars().map(|c| if c.is_alphanumeric() || "@._- ".contains(c) { c } else { '_' }).collect::<String>();
    let name = name.trim();
    if name.len() <= MAX_NAME {
        return name.to_string();
    }
    let mut end = MAX_NAME - 10;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}~{}", &name[..end], short_hash(s))
}

/// Eight hex characters identifying `s` exactly.
pub(crate) fn short_hash(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..8].to_string()
}

/// How a name compares on a case-insensitive file system (APFS, NTFS).
/// Non-ASCII letters all count as the same, so names that differ only in
/// Unicode normalization or case are caught too.
pub(crate) fn fold(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii() { c.to_ascii_lowercase() } else { '?' }).collect()
}

/// Names handed out in one directory, so two that the file system would
/// treat as the same (or a reserved one like `INDEX.md`) get a hash of
/// their source added instead of overwriting each other.
pub(crate) struct Names(HashSet<String>);

impl Names {
    pub(crate) fn reserving(reserved: &[&str]) -> Self {
        Names(reserved.iter().map(|r| fold(r)).collect())
    }

    /// `stem` + `ext` if free, else with `-<hash of source>` before `ext`.
    pub(crate) fn take(&mut self, stem: &str, ext: &str, source: &str) -> String {
        let plain = format!("{stem}{ext}");
        if self.0.insert(fold(&plain)) {
            return plain;
        }
        let hashed = format!("{stem}-{}{ext}", short_hash(source));
        self.0.insert(fold(&hashed));
        hashed
    }
}

/// Text from mail or events on one Markdown line: line breaks and other
/// control characters become spaces, so it can't start a heading, list item
/// or table row of its own.
pub(crate) fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// Untrusted text as a fenced block that nothing inside can close.
pub(crate) fn fenced(s: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in s.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}text\n{}\n{fence}\n", s.trim_end())
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
        std::fs::create_dir_all(&r)?;
        // An agent may have swapped these for symlinks; put real ones back.
        ensure_real_dir(&r, &r.join("proposals/inbox"))?;
        ensure_real_dir(&r, &r.join("proposals/processed"))?;
        // Mail is private: other accounts on this computer can't read it.
        #[cfg(unix)]
        std::fs::set_permissions(&r, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        write_if_changed(&self.root(), &r.join("README.md"), README.as_bytes())?;
        Ok(())
    }

    /// Remove a top-level section the settings turned off (`calendar`,
    /// `contacts`), so agents no longer see it.
    pub fn remove_section(&self, name: &str) -> Result<()> {
        let r = self.root();
        match remove_dir_all(&r, &r.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    fn account_allowed(&self, account_id: &str) -> bool {
        self.settings.account_ids.as_ref().is_none_or(|a| a.iter().any(|x| x == account_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_short_and_distinct() {
        let long = "é".repeat(200);
        let n = safe_name(&long);
        assert!(n.len() <= MAX_NAME, "{}", n.len());
        assert_ne!(n, safe_name(&format!("{long}x")));

        let mut names = Names::reserving(&["INDEX.md"]);
        assert_ne!(names.take("index", ".md", "mb1"), "index.md");
        assert_eq!(names.take("Event", ".ics", "Event"), "Event.ics");
        let other = names.take("event", ".ics", "event");
        assert!(other.starts_with("event-") && other.ends_with(".ics"), "{other}");
    }

    #[test]
    fn markdown_from_mail_cannot_break_out() {
        assert_eq!(one_line("a\r\n## b\u{2028}c"), "a  ## b c");
        let f = fenced("x\n````\ny");
        assert!(f.starts_with("`````text\n") && f.ends_with("\n`````\n"), "{f}");
    }
}
