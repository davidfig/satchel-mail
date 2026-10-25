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

use crate::ai_folder::{read_regular, root, write_atomic};
use crate::store::Store;

/// A proposal read from a file, before it is attributed to an agent.
#[derive(Debug, serde::Deserialize)]
pub struct DroppedProposal {
    /// Optional: files without a token come from the built-in AI folder agent.
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
    let dir = root(settings).join("proposals/inbox");
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json" || x == "md"))
        // Regular files only: a symlink could point anywhere on disk.
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
        .collect();
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
/// token is stripped; the name is kept so the agent can find it.
pub fn finish(settings: &MirrorSettings, path: &Path, outcome: Outcome) {
    let done = root(settings).join("proposals/processed");
    let _ = std::fs::create_dir_all(&done);
    let target = free_name(&done, path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    if let Ok(text) = read_regular(&root(settings), path) {
        let out = if is_markdown(path) {
            let mut fields = vec![("agent_token", None)];
            match &outcome {
                Outcome::Created(p) => fields.extend(status_fields(p, false)),
                Outcome::Error(e) => fields.extend([("status", Some("error".to_string())), ("error", Some(quote(e)))]),
            }
            set_fields(&text, &fields)
        } else {
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text }));
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
        let _ = write_atomic(&root(settings), &target, out.as_bytes());
    }
    let _ = std::fs::remove_file(path);
}

/// Rewrite the status of the processed file for `p`, if there is one.
/// Decisions are human-paced, so scanning the folder is fine.
pub fn update_status(settings: &MirrorSettings, p: &Proposal, edited: bool) {
    let dir = root(settings).join("proposals/processed");
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for path in rd.filter_map(|e| e.ok().map(|e| e.path())) {
        let Ok(text) = read_regular(&root(settings), &path) else { continue };
        if !text.contains(&p.id) {
            continue;
        }
        let out = if is_markdown(&path) {
            let Ok((fm, _)) = parse(&text) else { continue };
            if fm.str("proposal") != Some(p.id.as_str()) {
                continue;
            }
            set_fields(&text, &status_fields(p, edited))
        } else {
            let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            if v["result"]["ok"]["id"].as_str() != Some(p.id.as_str()) {
                continue;
            }
            v["status"] = status_word(p, edited).into();
            v["note"] = p.review_note.clone().into();
            serde_json::to_string_pretty(&v).unwrap_or_default()
        };
        let _ = write_atomic(&root(settings), &path, out.as_bytes());
        return;
    }
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
        ("note", p.review_note.as_deref().map(quote)),
        ("error", None),
    ]
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
