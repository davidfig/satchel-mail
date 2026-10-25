//! The audit log and settings.

use super::*;

impl Store {
    pub fn audit(&self, actor: &str, action: &str, detail: &serde_json::Value) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO audit (at, actor, action, detail_json) VALUES (?,?,?,?)",
                params![now_ms(), actor, action, detail.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditEntry>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT id, at, actor, action, detail_json FROM audit ORDER BY id DESC LIMIT ?")?;
            let rows = s
                .query_map([limit], |r| {
                    Ok(AuditEntry {
                        id: r.get(0)?,
                        at: ms_to_dt(r.get(1)?),
                        actor: r.get(2)?,
                        action: r.get(3)?,
                        detail: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.r(|c| {
            let v: Option<String> = c.query_row("SELECT value FROM settings WHERE key=?", [key], |r| r.get(0)).optional()?;
            Ok(v.and_then(|v| serde_json::from_str(&v).ok()))
        })
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, to_json(value)],
            )?;
            Ok(())
        })
    }
}
