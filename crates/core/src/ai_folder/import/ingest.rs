//! Reading the AI folder back: dropped proposal files and edited person files.

use crate::engine::{Caller, Engine};
use std::path::PathBuf;
use std::sync::Arc;

use satchel_api::*;


impl Engine {
    /// Turn files in the AI folder's `proposals/inbox/` into proposals.
    pub async fn ingest_dropped_proposals(self: &Arc<Self>) {
        let settings = self.settings();
        if !settings.mirror.enabled {
            return;
        }
        for path in crate::ai_folder::import::pending(&settings.mirror) {
            let created = match crate::ai_folder::import::read(&self.store, &settings.mirror, &path) {
                Ok(d) => self.ingest_one(&settings.mirror, d).await,
                Err(e) => Err(format!("{e:#}")),
            };
            let outcome = match &created {
                Ok(p) => crate::ai_folder::import::Outcome::Created(p),
                Err(e) => crate::ai_folder::import::Outcome::Error(e.clone()),
            };
            crate::ai_folder::import::finish(&settings.mirror, &path, outcome);
        }
        self.ingest_person_files(&settings.mirror).await;
    }

    /// Person files an agent added or edited in `contacts/people/` become
    /// contact proposals. The file moves to `proposals/processed/` like an
    /// inbox file, and the person's file is written again from the store.
    pub(crate) async fn ingest_person_files(self: &Arc<Self>, mirror: &MirrorSettings) {
        let dir = crate::ai_folder::root(mirror).join("contacts/people");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let Ok(text) = crate::ai_folder::read_regular(&crate::ai_folder::root(mirror), &path) else { continue };
            let last = self.contact_files.lock().unwrap().get(&name).cloned();
            let created = match crate::ai_folder::import::read_person(&self.store, &text, last.as_deref()) {
                Ok(None) => continue,
                Ok(Some(d)) => self.ingest_one(mirror, d).await,
                Err(e) => Err(format!("{e:#}")),
            };
            let outcome = match &created {
                Ok(p) => crate::ai_folder::import::Outcome::Created(p),
                Err(e) => crate::ai_folder::import::Outcome::Error(e.clone()),
            };
            crate::ai_folder::import::finish(mirror, &path, outcome);
            self.contact_files.lock().unwrap().remove(&name);
            self.contacts_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            self.mirror_wake.notify_one();
        }
    }

    pub(crate) async fn ingest_one(self: &Arc<Self>, mirror: &MirrorSettings, d: crate::ai_folder::import::DroppedProposal) -> std::result::Result<Proposal, String> {
        let agent = match &d.agent_token {
            Some(t) => self.authenticate_agent(t).ok_or("invalid or revoked agent_token")?,
            None => self.folder_agent(mirror).map_err(|e| format!("{e:#}"))?,
        };
        let rationale = match d.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(label) => format!("[{label}] {}", d.rationale),
            None => d.rationale,
        };
        let cmd = Command::CreateProposal { action: d.action, rationale, source_thread_ids: d.source_thread_ids };
        let v = self.handle(&Caller::Agent(agent), cmd).await.map_err(|e| e.message)?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}
