//! Accounts and their sync cursors.

use super::*;

impl Store {
    pub fn insert_account(&self, config: &AccountConfig) -> Result<Account> {
        let id = new_id();
        self.w(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
            let color = PALETTE[n as usize % PALETTE.len()];
            c.execute(
                "INSERT INTO accounts (id, kind, email, display_name, config_json, color, created_at) VALUES (?,?,?,?,?,?,?)",
                params![id, kind_str(config.kind()), config.email(), config.display_name(), to_json(config), color, now_ms()],
            )?;
            Ok(())
        })?;
        self.get_account(&id)?.ok_or_else(|| anyhow!("account vanished"))
    }

    fn account_row(r: &Row) -> rusqlite::Result<Account> {
        let config: String = r.get("config_json")?;
        let config: AccountConfig = serde_json::from_str(&config).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(Account {
            id: r.get("id")?,
            kind: config.kind(),
            email: r.get("email")?,
            display_name: r.get("display_name")?,
            config,
            color: r.get("color")?,
            sync: SyncStatus {
                state: parse_sync_state(&r.get::<_, String>("sync_state")?),
                last_success: r.get::<_, Option<i64>>("last_success")?.map(ms_to_dt),
                error: r.get("error")?,
                detail: r.get("detail")?,
            },
            contacts_need_reconnect: r.get("contacts_reconnect")?,
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT a.*, EXISTS (SELECT 1 FROM sync_cursors k WHERE k.account_id=a.id AND k.key='contacts_scope_missing') AS contacts_reconnect FROM accounts a ORDER BY created_at")?;
            let rows = s.query_map([], Self::account_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        self.r(|c| Ok(c.query_row("SELECT a.*, EXISTS (SELECT 1 FROM sync_cursors k WHERE k.account_id=a.id AND k.key='contacts_scope_missing') AS contacts_reconnect FROM accounts a WHERE id = ?", [id], Self::account_row).optional()?))
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM accounts WHERE id = ?", [id])?;
            c.execute("DELETE FROM threads WHERE account_id = ?", [id])?;
            Ok(())
        })
    }

    pub fn set_sync_status(&self, id: &str, st: &SyncStatus) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE accounts SET sync_state=?, last_success=?, error=?, detail=? WHERE id=?",
                params![sync_state_str(st.state), st.last_success.map(|d| d.timestamp_millis()), st.error, st.detail, id],
            )?;
            Ok(())
        })
    }

    pub fn update_account_config(&self, id: &str, config: &AccountConfig) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE accounts SET config_json=? WHERE id=?", params![to_json(config), id])?;
            Ok(())
        })
    }


    pub fn cursor(&self, account_id: &str, key: &str) -> Result<Option<String>> {
        self.r(|c| {
            Ok(c.query_row("SELECT value FROM sync_cursors WHERE account_id=? AND key=?", [account_id, key], |r| r.get(0))
                .optional()?)
        })
    }

    pub fn set_cursor(&self, account_id: &str, key: &str, value: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO sync_cursors (account_id, key, value) VALUES (?,?,?) ON CONFLICT(account_id, key) DO UPDATE SET value=excluded.value",
                params![account_id, key, value],
            )?;
            Ok(())
        })
    }

    pub fn delete_cursor(&self, account_id: &str, key: &str) -> Result<()> {
        self.w(|c| {
            c.execute("DELETE FROM sync_cursors WHERE account_id=? AND key=?", [account_id, key])?;
            Ok(())
        })
    }

    pub fn delete_cursors_with_prefix(&self, account_id: &str, prefix: &str) -> Result<()> {
        self.w(|c| {
            c.execute(
                "DELETE FROM sync_cursors WHERE account_id=? AND key LIKE ?",
                params![account_id, format!("{prefix}%")],
            )?;
            Ok(())
        })
    }
}
