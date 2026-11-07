//! The unread inbox count on the app's icon: the Dock badge on macOS (also
//! shown in Cmd-Tab), a taskbar overlay on Windows, and the launcher count on
//! Linux docks that support it. It's set here, not from the UI, because a
//! hidden webview can be suspended while the engine keeps syncing (0070, 0071).

#[cfg(any(windows, test))]
mod icon;
#[cfg(target_os = "linux")]
mod launcher;

use std::sync::Arc;
use std::time::Duration;

use satchel_api::{Command, CoreEvent, Mailbox, MailboxRole};
use satchel_core::{Caller, Engine};
use tauri::WebviewWindow;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// Sync sends events in bursts; wait this long so one burst is one count.
const SETTLE: Duration = Duration::from_millis(250);

/// Windows ignores an overlay set before the taskbar button exists, and doesn't
/// say so, so the first count is set again after this long.
const TASKBAR_READY: Duration = Duration::from_secs(3);

/// Keep the badge current until the engine shuts down.
pub fn start(window: WebviewWindow, engine: Arc<Engine>) {
    // Subscribed before the first count, so no change falls between the two.
    let mut rx = engine.subscribe();
    let mut display = Display::new(window);
    tauri::async_runtime::spawn(async move {
        let mut shown = None;
        let mut first = true;
        loop {
            match count(&engine).await {
                Ok(n) if shown != Some(n) => match display.show(n).await {
                    Ok(()) => shown = Some(n),
                    Err(e) => tracing::warn!("couldn't set the unread badge: {e}"),
                },
                Ok(_) => {}
                Err(e) => tracing::warn!("couldn't count unread mail for the badge: {e}"),
            }
            if cfg!(windows) && std::mem::take(&mut first) {
                tokio::time::sleep(TASKBAR_READY).await;
                shown = None;
                continue;
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

#[cfg(target_os = "linux")]
use launcher::Launcher as Display;

/// Where the count goes on hosts where Tauri can show it.
#[cfg(not(target_os = "linux"))]
struct Display(WebviewWindow);

#[cfg(not(target_os = "linux"))]
impl Display {
    fn new(window: WebviewWindow) -> Self {
        Self(window)
    }

    async fn show(&mut self, count: i64) -> Result<(), String> {
        #[cfg(windows)]
        let shown = self.0.set_overlay_icon((count > 0).then(|| tauri::image::Image::new_owned(icon::render(count), icon::SIZE, icon::SIZE)));
        #[cfg(not(windows))]
        let shown = self.0.set_badge_count((count > 0).then_some(count));
        shown.map_err(|e| e.to_string())
    }
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
