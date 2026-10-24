# 0025. New-mail notifications in the system notification center

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

New mail only showed up as a count in the sidebar and the window title, so
you had to keep looking at the app to notice it. Every desktop OS has a
notification center (macOS Notification Center, Windows Action Center, the
freedesktop notification daemon on Linux) and browsers expose the same thing
through the Notification API.

## Decision

The core decides what counts as new mail; the hosts only display it.

- During sync, `SyncCtx::put_record` notes each newly stored message that is
  unread, in an inbox, not a draft, not from the account itself, not from a
  sender marked as spam, and dated no earlier than a day before the account's
  last successful sync. Nothing is announced before an account's first sync
  finishes, so adding an account or a backfill of old mail never floods the
  notification center.
- When a sync pass ends, `messages_changed` sends one
  `CoreEvent::NewMail { account_id, messages }` with everything that arrived.
- The UI (`lib/notify.ts`) shows one notification per batch: the sender and
  subject for a single message, or "N new messages" with the senders. It
  stays quiet while the app window has focus.
- Display goes through `transport.notify()`: `tauri-plugin-notification` on
  desktop (limited to the permission-check and notify commands), the
  browser's `Notification` on the web, where clicking opens the conversation.
  Desktop notifications don't report clicks; clicking one brings the app
  forward.
- A General setting, `notify_new_mail`, turns it off. It defaults to on, and
  stored settings without it read as on.

## Consequences

- Hosts that don't show notifications (the CLI, MCP) can ignore the event.
- Rules that run after sync can still move an announced message; a sender
  with a "Never spam" rule whose mail the server put in Junk is moved to the
  inbox without a notification.
- Clicking a desktop notification doesn't open the conversation yet; that
  needs the plugin's action support or a native handler in the Tauri host.
