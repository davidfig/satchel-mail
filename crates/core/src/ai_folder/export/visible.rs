//! The folder's date limit, applied with `mail::visible`: each thread shows
//! only its messages inside the limit, and so does every index.

use super::*;
use crate::store::StoredMessage;

impl Mirror<'_> {
    /// The oldest date the folder shows, if the settings limit it.
    pub(super) fn cutoff(&self) -> Option<chrono::DateTime<Utc>> {
        self.settings.since_days.map(|d| Utc::now() - Duration::days(d as i64))
    }

    /// The thread's messages inside the date limit, oldest first.
    pub(super) fn visible_messages(&self, thread_id: &str) -> Result<Vec<StoredMessage>> {
        crate::mail::visible::visible_messages(self.store, thread_id, self.cutoff())
    }

    /// `t` as the folder may show it, or None if nothing is inside the limit.
    pub(super) fn visible_summary(&self, t: ThreadSummary) -> Result<Option<ThreadSummary>> {
        crate::mail::visible::visible_summary(self.store, t, self.cutoff())
    }
}
