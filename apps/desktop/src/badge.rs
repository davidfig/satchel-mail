//! The Dock badge: unread inbox mail, which macOS also shows in the Cmd-Tab
//! switcher. It's set here, not from the UI, because macOS can suspend a
//! hidden webview while the engine keeps syncing (0070).

use std::sync::Arc;
use std::time::Duration;

use satchel_api::{Command, CoreEvent, Mailbox, MailboxRole};
use satchel_core::{Caller, Engine};
use tauri::WebviewWindow;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Sync sends events in bursts; wait this long so one burst is one count.
const SETTLE: Duration = Duration::from_millis(250);

/// Keep the badge current until the engine shuts down.
pub fn start(window: WebviewWindow, engine: Arc<Engine>) {
    // Subscribed before the first count, so no change falls between the two.
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut shown = None;
        loop {
            match count(&engine).await {
                Ok(n) if shown != Some(n) => {
                    if let Err(e) = window.set_badge_count((n > 0).then_some(n)) {
                        tracing::warn!("couldn't set the Dock badge: {e}");
                    }
                    shown = Some(n);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("couldn't count unread mail for the Dock badge: {e}"),
            }
            loop {
                match rx.recv().await {
                    Ok(e) if changes_unread(&e) => break,
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => return,
                }
            }
            tokio::time::sleep(SETTLE).await;
            // The count below covers whatever arrived while we waited.
            while !matches!(rx.try_recv(), Err(TryRecvError::Empty | TryRecvError::Closed)) {}
        }
    });
}

async fn count(engine: &Arc<Engine>) -> Result<i64, String> {
    let value = engine.handle(&Caller::User, Command::ListMailboxes { account_id: None }).await.map_err(|e| e.message)?;
    let mailboxes: Vec<Mailbox> = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(inbox_unread(&mailboxes))
}

/// The same events that make the UI reload its mailbox counts.
fn changes_unread(event: &CoreEvent) -> bool {
    matches!(event, CoreEvent::AccountsChanged | CoreEvent::MailboxesChanged { .. } | CoreEvent::MessagesChanged { .. } | CoreEvent::DataChanged)
}

/// Unread mail across every account's inbox, matching the sidebar's Inbox.
fn inbox_unread(mailboxes: &[Mailbox]) -> i64 {
    mailboxes.iter().filter(|m| m.role == MailboxRole::Inbox).map(|m| i64::from(m.unread)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailbox(role: MailboxRole, unread: u32) -> Mailbox {
        Mailbox { id: String::new(), account_id: String::new(), name: String::new(), path: String::new(), role, is_label: false, unread, total: unread }
    }

    #[test]
    fn counts_only_inboxes_across_accounts() {
        let boxes = [mailbox(MailboxRole::Inbox, 3), mailbox(MailboxRole::Inbox, 2), mailbox(MailboxRole::Junk, 40), mailbox(MailboxRole::Drafts, 7)];
        assert_eq!(inbox_unread(&boxes), 5);
    }

    #[test]
    fn no_unread_mail_counts_zero() {
        assert_eq!(inbox_unread(&[mailbox(MailboxRole::Inbox, 0)]), 0);
        assert_eq!(inbox_unread(&[]), 0);
    }
}
