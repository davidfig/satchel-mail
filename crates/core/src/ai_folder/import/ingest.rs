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
            let file = crate::ai_folder::import::finish(&settings.mirror, &path, outcome);
            self.record_processed_file(created.as_ref().ok(), file);
        }
        self.ingest_person_files(&settings.mirror).await;
    }

    /// Person files an agent added or edited in `contacts/people/` become
    /// contact proposals. The file moves to `proposals/processed/` like an
    /// inbox file, and the person's file is written again from the store.
    pub(crate) async fn ingest_person_files(self: &Arc<Self>, mirror: &MirrorSettings) {
        if !mirror.include_contacts {
            return;
        }
        let root = crate::ai_folder::root(mirror);
        let Ok(files) = crate::ai_folder::list_files(&root, &root.join("contacts/people")) else { return };
        let mut paths: Vec<PathBuf> = files.into_iter().filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let created = match crate::ai_folder::read_regular(&root, &path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                // A hard link or an oversized file: reported back, not read.
                Err(e) => Err(format!("couldn't read {name}: {e}")),
                Ok(text) => {
                    let last = self.contact_files.lock().unwrap().get(&name).cloned();
                    match crate::ai_folder::import::read_person(&self.store, &text, last.as_deref()) {
                        Ok(None) => continue,
                        Ok(Some(d)) => self.ingest_one(mirror, d).await,
                        Err(e) => Err(format!("{e:#}")),
                    }
                }
            };
            let outcome = match &created {
                Ok(p) => crate::ai_folder::import::Outcome::Created(p),
                Err(e) => crate::ai_folder::import::Outcome::Error(e.clone()),
            };
            let file = crate::ai_folder::import::finish(mirror, &path, outcome);
            self.record_processed_file(created.as_ref().ok(), file);
            self.contact_files.lock().unwrap().remove(&name);
            self.contacts_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            self.mirror_wake.notify_one();
        }
    }

    fn record_processed_file(&self, p: Option<&Proposal>, file: Option<String>) {
        if let (Some(p), Some(file)) = (p, file)
            && let Err(e) = self.store.set_proposal_file(&p.id, &file)
        {
            tracing::warn!("AI folder: recording the processed file for {}: {e:#}", p.id);
        }
    }

    pub(crate) async fn ingest_one(self: &Arc<Self>, mirror: &MirrorSettings, d: crate::ai_folder::import::DroppedProposal) -> std::result::Result<Proposal, String> {
        // Folder files always come from the AI folder agent. A token in one
        // could be read by anything else with access to the folder before
        // Satchel picks it up, so it's refused, not quietly ignored.
        if d.agent_token.is_some() {
            return Err(crate::ai_folder::import::TOKEN_REFUSED.into());
        }
        let agent = self.folder_agent(mirror).map_err(|e| format!("{e:#}"))?;
        let rationale = match d.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(label) => format!("[{label}] {}", d.rationale),
            None => d.rationale,
        };
        let cmd = Command::CreateProposal { action: d.action, rationale, source_thread_ids: d.source_thread_ids };
        let v = self.handle(&Caller::Agent(agent), cmd).await.map_err(|e| e.message)?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}
