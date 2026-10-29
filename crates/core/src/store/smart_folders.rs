//! Smart folders: saved searches, in sidebar order.

use super::*;

fn badge_str(b: SmartBadge) -> &'static str {
    match b {
        SmartBadge::Unread => "unread",
        SmartBadge::Total => "total",
        SmartBadge::None => "none",
    }
}

fn parse_badge(s: &str) -> SmartBadge {
    match s {
        "total" => SmartBadge::Total,
        "none" => SmartBadge::None,
        _ => SmartBadge::Unread,
    }
}

fn row(r: &Row) -> rusqlite::Result<SmartFolder> {
    Ok(SmartFolder {
        id: r.get("id")?,
        name: r.get("name")?,
        query: r.get("query")?,
        badge: parse_badge(&r.get::<_, String>("badge")?),
        count: 0,
    })
}

impl Store {
    pub fn list_smart_folders(&self) -> Result<Vec<SmartFolder>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM smart_folders ORDER BY position, created_at")?;
            Ok(s.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn get_smart_folder(&self, id: &str) -> Result<Option<SmartFolder>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM smart_folders WHERE id=?", [id], row).optional()?))
    }

    /// Insert (empty id: a new id, placed last) or update name, query and badge.
    pub fn save_smart_folder(&self, f: &SmartFolder) -> Result<String> {
        self.w(|c| {
            if f.id.is_empty() {
                let id = new_id();
                c.execute(
                    "INSERT INTO smart_folders (id, name, query, badge, position, created_at)
                     VALUES (?,?,?,?,(SELECT COALESCE(MAX(position),-1)+1 FROM smart_folders),?)",
                    params![id, f.name, f.query, badge_str(f.badge), now_ms()],
                )?;
                Ok(id)
            } else {
                let n = c.execute(
                    "UPDATE smart_folders SET name=?, query=?, badge=? WHERE id=?",
                    params![f.name, f.query, badge_str(f.badge), f.id],
                )?;
                if n == 0 {
                    return Err(anyhow!("smart folder not found"));
                }
                Ok(f.id.clone())
            }
        })
    }

    pub fn delete_smart_folder(&self, id: &str) -> Result<bool> {
        self.w(|c| Ok(c.execute("DELETE FROM smart_folders WHERE id=?", [id])? > 0))
    }

    /// Number the folders in `ids` order; any not listed keep their order after them.
    pub fn reorder_smart_folders(&self, ids: &[String]) -> Result<()> {
        self.w(|c| {
            let tx = c.transaction()?;
            let rest: Vec<String> = {
                let mut s = tx.prepare("SELECT id FROM smart_folders ORDER BY position, created_at")?;
                s.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?
            };
            let order = ids.iter().filter(|id| rest.contains(id)).chain(rest.iter().filter(|id| !ids.contains(id)));
            for (i, id) in order.enumerate() {
                tx.execute("UPDATE smart_folders SET position=? WHERE id=?", params![i as i64, id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}
