//! `sync.json`: how fresh the folder is, so an agent can tell stale or
//! half-written data from current.

use super::*;

impl Mirror<'_> {
    /// Write `sync.json`. `complete` is false while threads are still waiting
    /// to be written (a first export, or a rewrite after a settings change).
    pub fn write_status(&self, complete: bool, pending_threads: usize) -> Result<()> {
        let mut accounts = vec![];
        for acc in self.store.list_accounts()? {
            if !self.account_allowed(&acc.id) {
                continue;
            }
            accounts.push(serde_json::json!({
                "account": acc.email,
                "folder": format!("mail/{}", safe_name(&acc.email)),
                "state": acc.sync.state,
                "last_sync": acc.sync.last_success,
                "error": acc.sync.error,
            }));
        }
        let mut status = serde_json::json!({
            "complete": complete,
            "pending_threads": pending_threads,
            "since_days": self.settings.since_days,
            "accounts": accounts,
        });
        // Attachments arrive in the background, after their threads (0078).
        if let Some(since) = self.attachment_window() {
            status["attachment_days"] = serde_json::json!(self.settings.attachment_days);
            status["pending_attachments"] = serde_json::json!(self.store.ai_attachments_pending(since.map(|d| d.timestamp_millis()))?);
        }
        write_if_changed(&self.root(), &self.root().join("sync.json"), serde_json::to_vec_pretty(&status)?.as_slice())
    }
}
