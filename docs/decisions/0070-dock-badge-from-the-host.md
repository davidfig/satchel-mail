# 0070. The Dock badge is set by the host, not the UI

- **Status:** Accepted
- **Date:** 2026-11-07

## Context
On macOS, unread inbox mail should show on the app icon, in the Dock and in the
Cmd-Tab switcher, which shows the same badge. The UI already computes this
number for the window title, so setting the badge there would be the smallest
change. But the badge matters most when the window isn't in front, and macOS can
throttle or suspend a minimized or hidden webview until it's visible again. The
engine keeps syncing in the host process either way.

## Decision
The desktop host keeps the badge (`apps/desktop/src/badge.rs`). It subscribes to
engine events and, when mail or mailbox counts change, runs `ListMailboxes`
through `Engine::handle` and sets the unread inbox total, waiting briefly so a
burst of sync events is one count. It's macOS only for now.

We didn't turn off webview throttling instead: that keeps the whole UI running
in the background, using more power, just to update a number.

## Consequences
- The badge doesn't depend on the webview's state.
- The count is computed in two places, the host and the sidebar. Both add up
  `unread` on inbox-role mailboxes and must stay in step.
- The badge only exists while the app runs. It updates only as often as sync
  sees changes, and macOS's per-app badge setting can hide it.
