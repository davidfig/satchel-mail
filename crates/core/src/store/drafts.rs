//! Local drafts.

use super::*;

impl Store {
    pub fn save_draft(&self, d: &Draft) -> Result<String> {
        let id = d.id.clone().unwrap_or_else(new_id);
        let mut d = d.clone();
        d.id = Some(id.clone());
        self.w(|c| {
            c.execute(
                "INSERT INTO drafts (id, account_id, draft_json, updated_at) VALUES (?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id, draft_json=excluded.draft_json, updated_at=excluded.updated_at",
                params![id, d.account_id, to_json(&d), now_ms()],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// The local id of an account's message with this Message-ID header.
    pub fn message_id_by_header(&self, account_id: &str, message_id_hdr: &str) -> Result<Option<String>> {
        self.r(|c| Ok(c.query_row("SELECT id FROM messages WHERE account_id=? AND message_id_hdr=? LIMIT 1", [account_id, message_id_hdr], |r| r.get(0)).optional()?))
    }

    pub fn get_draft(&self, id: &str) -> Result<Option<(Draft, Option<String>)>> {
        self.r(|c| {
            Ok(c.query_row("SELECT draft_json, server_message_id FROM drafts WHERE id=?", [id], |r| {
                let j: String = r.get(0)?;
                Ok((serde_json::from_str(&j).unwrap_or_default(), r.get(1)?))
            })
            .optional()?)
        })
    }

    pub fn set_draft_server_message(&self, id: &str, message_id: Option<&str>) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE drafts SET server_message_id=? WHERE id=?", params![message_id, id])?;
            Ok(())
        })
    }

    pub fn delete_draft(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM drafts WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn list_drafts(&self) -> Result<Vec<Draft>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT draft_json FROM drafts ORDER BY updated_at DESC")?;
            let rows = s
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|j| j.ok().and_then(|j| serde_json::from_str(&j).ok()))
                .collect();
            Ok(rows)
        })
    }
}
