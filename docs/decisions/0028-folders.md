# 0028. Folders: managed in Satchel, pushed through the op queue

- **Status:** Accepted
- **Date:** 2026-10-24

## Context
Folders (and Gmail labels) could only be made, renamed or deleted in another
client or on the web. Folder names work differently per protocol: IMAP names
are full paths that change when a folder moves (and may need modified UTF-7,
a different delimiter, or an `INBOX.` prefix), JMAP and Microsoft Graph use
stable ids with a parent, and Gmail nests labels by name (`Parent/Child` is a
separate label from `Parent`).

## Decision
- Three user-only commands: `CreateMailbox`, `UpdateMailbox` (rename and/or
  move under another folder) and `DeleteMailbox`. Agents can't use them
  (deleting is never an agent action, see 0006).
- Changes apply locally at once and reach the server through the op queue
  (`mailbox_create`, `mailbox_update`, `mailbox_delete`), like message changes.
  Each provider implements `ResolvedOp::{Create,Update,Delete}Mailbox` in its
  own terms: IMAP CREATE/RENAME/DELETE (renaming reports the new names of the
  folder and everything under it), JMAP `Mailbox/set`, Gmail label create/
  patch/delete (renaming patches nested labels too), Graph `mailFolders`
  (move and rename).
- A new folder gets a `pending:` remote id until the server creates it, so mail
  can be moved into it straight away. Moves and label changes record the
  folder's local id and look up its remote id when they run, so they wait for a
  pending folder and survive renames. Sync never prunes a pending folder; if
  the create fails for good, the placeholder is dropped.
- Only your own folders can change, not Inbox, Sent and the other system
  folders. Names can't be empty, contain `/`, or repeat a sibling's (ignoring
  case); a folder can't go inside itself.
- **Deleting never loses mail.** A folder's messages move to Trash first, then
  the folder goes. A Gmail label is just removed from its messages. A folder
  with folders inside it can't be deleted until those are moved or deleted.
  Three things make that hold, found by losing a test message on the local
  IMAP server while building this:
  - The op queue (0008) runs ops strictly in the order they were queued
    (insertion order breaks ties within a millisecond), and an op waiting to
    retry holds back everything queued after it for that account. Sends stay
    independent: they neither wait nor block.
  - A queued op takes each message's server identity from the store when it
    runs, not from when it was queued, so a second move (into a folder, then on
    to Trash) finds the message where the first put it. Before, it looked in the
    old place and silently did nothing on IMAP.
  - As a last guard, IMAP and Graph refuse to delete a folder that still has
    messages on the server (JMAP's `onDestroyRemoveEmails: false` does the
    same). A refused delete fails the op, the account resyncs, and the folder
    comes back with its mail.
- The sidebar shows folders as a tree. New folder is on each account's header
  (the + button and the right-click menu), on mailbox menus, and at the end of
  Move to…; folders' menus add New folder inside, Rename or move, and Delete.

## Consequences
- Folder changes work offline and retry like everything else, and a failed one
  shows up as a failed op, followed by a full resync.
- An op stuck retrying now delays the account's later ops until it succeeds or
  gives up (8 attempts, a few minutes), where before later ops overtook it.
  That's slower when the server is unhappy, but nothing runs out of order.
- IMAP renames keep message UIDs (RFC 3501), so a renamed folder is matched by
  UID and nothing is downloaded again. Messages keep remote ids that name the
  old folder; they are only labels, and fetching uses the folder memberships.
- Tested against the fake provider (engine tests) and the local Stalwart IMAP
  server. JMAP, Gmail and Graph folder ops follow their documentation but
  haven't been run against real servers. Graph is assumed to keep a folder's id
  when it moves.
- Not done: dragging folders or conversations in the sidebar, and choosing
  which folders sync.
