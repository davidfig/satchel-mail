//! Smart folders: saved searches across every folder of every account (0065).

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::{Caller, Engine};
use crate::error::{invalid, not_found};
/// The longest query a smart folder may save, in characters.
const MAX_QUERY: usize = 2000;

impl Engine {
    /// A smart folder's name and query can describe any account's mail, so
    /// only a caller who sees every account may see them.
    pub(crate) fn may_see_smart_folders(&self, caller: &Caller) -> Result<bool> {
        let Some(allowed) = self.allowed_accounts(caller) else { return Ok(true) };
        Ok(self.store.list_accounts()?.iter().all(|a| allowed.contains(&a.id)))
    }

    /// Smart folders in order, each with the count its badge shows. The counts
    /// run in the background (they rerun as mail syncs, and shouldn't hold up
    /// what the user is doing), and one that fails shows nothing instead of
    /// taking the whole list down with it.
    pub(crate) fn list_smart_folders(&self, caller: &Caller) -> Result<Vec<SmartFolder>> {
        if !self.may_see_smart_folders(caller)? {
            return Ok(vec![]);
        }
        crate::store::background(|| {
            let mut folders = self.store.list_smart_folders()?;
            for f in folders.iter_mut().filter(|f| f.badge != SmartBadge::None) {
                f.count = self.smart_folder_count(caller, f).unwrap_or_else(|e| {
                    tracing::warn!("smart folder {}: {e:#}", f.id);
                    0
                });
            }
            Ok(folders)
        })
    }

    fn smart_folder_count(&self, caller: &Caller, f: &SmartFolder) -> Result<u32> {
        let q = ThreadQuery { smart_folder_id: Some(f.id.clone()), ..Default::default() };
        let mut filter = self.thread_filter(caller, &q)?;
        filter.has_unread = f.badge == SmartBadge::Unread;
        self.store.count_threads(&filter)
    }

    /// Only the user changes smart folders (agents read them).
    fn may_change(&self, caller: &Caller) -> Result<()> {
        self.require_user(caller)?;
        self.require_full()
    }

    pub(crate) fn save_smart_folder(&self, caller: &Caller, mut f: SmartFolder) -> Result<SmartFolder> {
        self.may_change(caller)?;
        f.name = f.name.trim().to_string();
        f.query = f.query.split_whitespace().collect::<Vec<_>>().join(" ");
        if f.name.is_empty() {
            return Err(invalid("a smart folder needs a name"));
        }
        if f.name.chars().count() > 100 {
            return Err(invalid("that name is too long"));
        }
        if f.query.chars().count() > MAX_QUERY {
            return Err(invalid("that search is too long"));
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
        self.store.audit(&caller.actor(), what, &json!({ "name": f.name, "query": f.query }))?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(f)
    }

    pub(crate) fn delete_smart_folder(&self, caller: &Caller, id: &str) -> Result<()> {
        self.may_change(caller)?;
        let f = self.store.get_smart_folder(id)?.ok_or_else(|| not_found("smart folder"))?;
        self.store.delete_smart_folder(id)?;
        self.store.audit(&caller.actor(), "smart_folder_deleted", &json!({ "name": f.name }))?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(())
    }

    pub(crate) fn reorder_smart_folders(&self, caller: &Caller, ids: &[Id]) -> Result<()> {
        self.may_change(caller)?;
        self.store.reorder_smart_folders(ids)?;
        self.emit(CoreEvent::SmartFoldersChanged);
        Ok(())
    }
}
