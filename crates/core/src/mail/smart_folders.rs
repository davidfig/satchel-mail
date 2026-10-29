//! Smart folders: saved searches across every folder of every account (0039).

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::{Caller, Engine};
use crate::error::{invalid, not_found};
use crate::store::{MsgCond, MsgTerm};

impl Engine {
    /// Smart folders in order, each with the count its badge shows. Counts
    /// respect the caller's access (an agent sees only its accounts).
    pub(crate) fn list_smart_folders(&self, caller: &Caller) -> Result<Vec<SmartFolder>> {
        let mut folders = self.store.list_smart_folders()?;
        for f in folders.iter_mut().filter(|f| f.badge != SmartBadge::None) {
            let q = ThreadQuery { smart_folder_id: Some(f.id.clone()), ..Default::default() };
            let mut filter = self.thread_filter(caller, &q)?;
            if f.badge == SmartBadge::Unread {
                filter.conds.push(MsgCond::new(MsgTerm::Unread));
            }
            f.count = self.store.count_threads(&filter)?;
        }
        Ok(folders)
    }

    pub(crate) fn save_smart_folder(&self, mut f: SmartFolder, actor: &str) -> Result<SmartFolder> {
        f.name = f.name.trim().to_string();
        f.query = f.query.split_whitespace().collect::<Vec<_>>().join(" ");
        if f.name.is_empty() {
            return Err(invalid("a smart folder needs a name"));
        }
        if f.name.chars().count() > 100 {
            return Err(invalid("that name is too long"));
        }
        let p = crate::mail::search::parse_query(&f.query);
        if p.conds.is_empty() && p.accounts.is_empty() {
            return Err(invalid("a smart folder needs something to search for"));
        }
        let existing = self.store.list_smart_folders()?;
        if !f.id.is_empty() && !existing.iter().any(|x| x.id == f.id) {
            return Err(not_found("smart folder"));
        }
        if existing.iter().any(|x| x.id != f.id && x.name.eq_ignore_ascii_case(&f.name)) {
            return Err(invalid(format!("there is already a smart folder called {}", f.name)));
        }
        let created = f.id.is_empty();
        f.id = self.store.save_smart_folder(&f)?;
        let what = if created { "smart_folder_created" } else { "smart_folder_changed" };
        self.store.audit(actor, what, &json!({ "name": f.name, "query": f.query }))?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(f)
    }

    pub(crate) fn delete_smart_folder(&self, id: &str, actor: &str) -> Result<()> {
        let f = self.store.get_smart_folder(id)?.ok_or_else(|| not_found("smart folder"))?;
        self.store.delete_smart_folder(id)?;
        self.store.audit(actor, "smart_folder_deleted", &json!({ "name": f.name }))?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(())
    }

    pub(crate) fn reorder_smart_folders(&self, ids: &[Id]) -> Result<()> {
        self.store.reorder_smart_folders(ids)?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(())
    }
}
