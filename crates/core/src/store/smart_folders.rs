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

    /// Insert or update a folder with a given id and place (sync from
    /// another computer, 0080). With no place, a new one goes last.
    pub fn put_smart_folder(&self, f: &SmartFolder, position: Option<i64>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO smart_folders (id, name, query, badge, position, created_at)
                 VALUES (?,?,?,?,COALESCE(?, (SELECT COALESCE(MAX(position),-1)+1 FROM smart_folders)),?)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, query=excluded.query, badge=excluded.badge,
                   position=COALESCE(?, position)",
                params![f.id, f.name, f.query, badge_str(f.badge), position, now_ms(), position],
            )?;
            Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_folders_keep_their_order() {
        let s = Store::open_in_memory().unwrap();
        let save = |name: &str| s.save_smart_folder(&SmartFolder { name: name.into(), query: "is:unread".into(), ..Default::default() }).unwrap();
        let (a, b, c) = (save("A"), save("B"), save("C"));
        let names = || s.list_smart_folders().unwrap().into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(names(), ["A", "B", "C"]);
        s.reorder_smart_folders(&[c.clone(), a.clone()]).unwrap();
        assert_eq!(names(), ["C", "A", "B"]);
        s.save_smart_folder(&SmartFolder { id: b.clone(), name: "Bee".into(), query: "is:flagged".into(), badge: SmartBadge::Total, count: 0 }).unwrap();
        let bee = s.get_smart_folder(&b).unwrap().unwrap();
        assert_eq!((bee.name.as_str(), bee.query.as_str(), bee.badge), ("Bee", "is:flagged", SmartBadge::Total));
        assert!(s.delete_smart_folder(&a).unwrap());
        assert_eq!(names(), ["C", "Bee"]);
        assert!(s.save_smart_folder(&SmartFolder { id: a, name: "gone".into(), ..Default::default() }).is_err());
    }
}
