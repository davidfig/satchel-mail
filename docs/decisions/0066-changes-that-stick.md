# 0066. A change on screen stays there: commit it with its op, and hold list reads

- **Status:** Accepted
- **Date:** 2026-11-05

## Context
A deleted conversation left the list, came back, then left again. Holding back
sync while an op is queued (b34352e) closed one way this happens, but left
others, each of which looks the same on screen:

- Gmail keeps the Spam label when a message is deleted unless told otherwise,
  and we only removed Inbox, so a message deleted from Spam stayed in Spam
  until the server's answer synced down.
- Gmail also keeps a trashed message's other labels, and hides it under them.
  We didn't, so it stayed listed under Sent or its label for good.
- The local change was written before its op was queued, and sync checked for a
  queued op before writing. A sync could slip into either gap and undo the
  change, or store a message again while it was being deleted for good.
- The UI took rows off the list, then told the core. A refresh in between (a
  sync tick; the core's write can wait behind sync's) read the old rows.

## Decision
- **A local change and its op commit together** (`Store::enqueue_change`), and
  sync's check for a queued op happens inside its write (`sync_memberships`,
  `sync_flags`). A sync then lands wholly before the change, which overwrites
  it, or wholly after, and leaves the message alone. Every optimistic change to
  a message goes through here.
- **The list doesn't read while a removal is being written**
  (`ThreadWindow.holdReads`). Reads already under way are dropped, and new ones
  wait until the core answers. If the write fails, the refresh after it brings
  the rows back.
- **Only Trash and Spam list what's in them.** Deleting takes a message out of
  Spam as well as the Inbox, and listings under other labels leave out mail in
  Trash or Spam.
- **The reader moves on.** Deleting, archiving or otherwise moving the open
  conversation opens the one that took its place, in every folder, the same as
  pressing the next-conversation key (so it's marked read). The exception is
  Drafts, where opening a draft starts editing it.

## Consequences
Optimistic changes have one path, tested in `crates/core/tests/deleting.rs` and
`ui/test/removing.test.ts`; the races there write from another thread throughout
the action. A slow core write now delays list refreshes rather than showing old
rows. There's no e2e browser test yet: the UI tests run the real `act` and
`ThreadWindow` against a fake server.
