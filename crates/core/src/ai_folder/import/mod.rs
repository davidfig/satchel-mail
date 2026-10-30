//! Proposal files in the AI folder (`proposals/inbox/`).
//!
//! Agents propose changes by writing a Markdown file with a front matter
//! header (or a JSON file in the API's shape). The engine ingests each file
//! as a proposal and moves it to `proposals/processed/`, where its `status:`
//! is kept up to date as the user decides.

mod front_matter;
mod ingest;
mod markdown;
mod person;
mod resolve;
#[cfg(test)]
mod tests;

pub use front_matter::*;
pub use person::*;
use markdown::*;
use resolve::*;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
use satchel_api::*;

use crate::ai_folder::{list_files, read_regular, remove_file, root, write_atomic};
use crate::store::Store;

/// Why a folder file with an agent token is refused.
pub const TOKEN_REFUSED: &str = "agent tokens aren't accepted in AI folder files: anything else with access to the folder can read them. \
Remove `agent_token:` to propose as the AI folder agent, or send the proposal over MCP (`satchel mcp`) or /api/agent/rpc. \
This token has been exposed; ask the user to revoke it and connect a new one (Settings → AI agents).";

/// A proposal read from a file. Files always come from the built-in AI
/// folder agent.
#[derive(Debug, serde::Deserialize)]
pub struct DroppedProposal {
    /// Only read to refuse the file: tokens aren't accepted in the folder.
    #[serde(default)]
    pub agent_token: Option<String>,
    /// A self-declared name ("Claude"), shown in Review but not trusted.
    #[serde(default)]
    pub agent: Option<String>,
    pub action: ProposalAction,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub source_thread_ids: Vec<String>,
}

pub fn pending(settings: &MirrorSettings) -> Vec<PathBuf> {
    let r = root(settings);
    // Regular files only, listed without following a symlinked `proposals/`
    // or `inbox/`: through one, ingesting would delete files anywhere on disk.
    let files = match list_files(&r, &r.join("proposals/inbox")) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(e) => {
            tracing::warn!("AI folder: not reading proposals/inbox: {e}");
            return vec![];
        }
    };
    let mut v: Vec<PathBuf> = files.into_iter().filter(|p| p.extension().is_some_and(|x| x == "json" || x == "md")).collect();
    v.sort();
    v
}

pub(crate) fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "md")
}

/// Read one inbox file. Markdown is resolved against the store (thread ids to
/// message ids, mailbox and calendar names to ids).
pub fn read(store: &Store, settings: &MirrorSettings, path: &Path) -> Result<DroppedProposal> {
    let text = read_regular(&root(settings), path)?;
    refuse_tokens(&text)?;
    if is_markdown(path) {
        from_markdown(store, &text)
    } else {
        Ok(serde_json::from_str(&text)?)
    }
}

/// The result of ingesting a file, written back for the agent to read.
pub enum Outcome<'a> {
    Created(&'a Proposal),
    Error(String),
}

/// Move an ingested file to `proposals/processed/` with its outcome. The
/// token is stripped; the name is kept so the agent can find it. A file that
/// couldn't be read (a link, too large) gets a result without its content.
/// Returns the processed file's name, which the caller records on the
/// proposal so status updates go to that file and not one an agent planted.
pub fn finish(settings: &MirrorSettings, path: &Path, outcome: Outcome) -> Option<String> {
    let r = root(settings);
    let done = r.join("proposals/processed");
    let target = free_name(&done, path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let md = is_markdown(path);
    let text = read_regular(&r, path).unwrap_or_else(|_| if md { "---\n---\n".into() } else { "{}".into() });
    let out = if md {
        let mut fields = vec![("agent_token", None)];
        match &outcome {
            Outcome::Created(p) => fields.extend(status_fields(p, false)),
            Outcome::Error(e) => fields.extend([("status", Some("error".to_string())), ("error", Some(quote(e)))]),
        }
        set_fields(&text, &fields)
    } else {
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text }));
        if !v.is_object() {
            v = serde_json::json!({ "raw": v });
        }
        if let Some(o) = v.as_object_mut() {
            o.remove("agent_token");
            let result = match &outcome {
                Outcome::Created(p) => serde_json::json!({ "ok": p }),
                Outcome::Error(e) => serde_json::json!({ "error": e }),
            };
            o.insert("result".into(), result);
            if let Outcome::Created(p) = &outcome {
                o.insert("status".into(), status_word(p, false).into());
            }
        }
        serde_json::to_string_pretty(&v).unwrap_or_default()
    };
    // Whatever shape the file had, no token survives into processed/, where
    // every agent can read it.
    let written = match write_atomic(&r, &target, scrub_tokens(&out).as_bytes()) {
        Ok(()) => target.file_name().map(|n| n.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::warn!("AI folder: writing {}: {e:#}", target.display());
            None
        }
    };
    // Removed relative to the directory opened without following links, so
    // a symlinked inbox can't turn this into a delete elsewhere.
    if let Err(e) = remove_file(&r, path) {
        tracing::warn!("AI folder: removing {}: {e}", path.display());
    }
    written
}

/// Rewrite the status of `p` in `file`, the processed file Satchel wrote
/// for it (recorded on the proposal), if it still names the proposal.
pub fn update_status(settings: &MirrorSettings, p: &Proposal, file: &str, edited: bool) {
    let r = root(settings);
    if file.contains(['/', '\\']) || file.starts_with('.') {
        return;
    }
    let path = r.join("proposals/processed").join(file);
    let Ok(text) = read_regular(&r, &path) else { return };
    let out = if is_markdown(&path) {
        let Ok((fm, _)) = parse(&text) else { return };
        if fm.str("proposal") != Some(p.id.as_str()) {
            return;
        }
        set_fields(&text, &status_fields(p, edited))
    } else {
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if v["result"]["ok"]["id"].as_str() != Some(p.id.as_str()) {
            return;
        }
        v["status"] = status_word(p, edited).into();
        if let Some(o) = v.as_object_mut() {
            o.remove("note");
        }
        v["error"] = satchel_reason(p).into();
        serde_json::to_string_pretty(&v).unwrap_or_default()
    };
    let _ = write_atomic(&r, &path, scrub_tokens(&out).as_bytes());
}

/// Refuse a file that carries an agent token anywhere, in whatever field.
pub(crate) fn refuse_tokens(text: &str) -> Result<()> {
    if scrub_tokens(text) != text {
        bail!(TOKEN_REFUSED);
    }
    Ok(())
}

/// Replace anything that looks like an agent token (`stk_…`) with a marker.
pub(crate) fn scrub_tokens(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find("stk_").map(|i| at + i) {
        let n = s[i + 4..].bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-').count();
        out.push_str(&s[at..i]);
        out.push_str(if n == 0 { &s[i..i + 4] } else { "[agent_token removed]" });
        at = i + 4 + n;
    }
    out.push_str(&s[at..]);
    out
}

pub(crate) fn status_word(p: &Proposal, edited: bool) -> &'static str {
    match p.status {
        ProposalStatus::Pending => "pending",
        ProposalStatus::Approved | ProposalStatus::Applied if edited => "edited",
        ProposalStatus::Approved | ProposalStatus::Applied => "approved",
        ProposalStatus::Rejected => "rejected",
        ProposalStatus::Stale => "stale",
        ProposalStatus::Failed => "failed",
    }
}

pub(crate) fn status_fields(p: &Proposal, edited: bool) -> Vec<(&'static str, Option<String>)> {
    vec![
        ("status", Some(status_word(p, edited).to_string())),
        ("proposal", Some(p.id.clone())),
        ("summary", Some(quote(&p.summary))),
        ("warnings", (!p.warnings.is_empty()).then(|| format!("[{}]", p.warnings.iter().map(|w| quote(w)).collect::<Vec<_>>().join(", ")))),
        ("note", None),
        ("error", satchel_reason(p).as_deref().map(quote)),
    ]
}

/// Why a proposal went stale or failed, which Satchel wrote. The user's
/// review note is never copied into the folder: `processed/` is shared with
/// every agent, so a note there could have been written by any of them.
fn satchel_reason(p: &Proposal) -> Option<String> {
    match p.status {
        ProposalStatus::Stale | ProposalStatus::Failed => p.review_note.clone(),
        _ => None,
    }
}

pub(crate) fn free_name(dir: &Path, name: String) -> PathBuf {
    // Taken means anything there, a dangling symlink included.
    let free = |p: &PathBuf| std::fs::symlink_metadata(p).is_err();
    let first = dir.join(&name);
    if free(&first) {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), format!(".{e}"))).unwrap_or((name.clone(), String::new()));
    (2..).map(|n| dir.join(format!("{stem}-{n}{ext}"))).find(free).unwrap()
}
