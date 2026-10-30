# 0039. Newer mail first; older mail syncs over time

- **Status:** Accepted; per-pass budgets updated by 0047
- **Date:** 2026-10-30

## Context
A new account can have years of mail. What the user needs first is the
recent part: today's inbox, last week's sent mail. Providers did not all
fetch it first:

- IMAP synced one folder at a time, fetching every old header in a folder
  before moving to the next, so the Sent folder's recent mail waited behind
  the Inbox's whole history.
- JMAP listed every message id in the account before fetching any headers.
- Outlook's per-folder delta came back in no promised order, so an old
  message could arrive before a recent one, and one folder's history
  blocked the next folder.
- Gmail already synced a recent window and then backfilled 1000 older
  messages per pass, but it waited a full poll interval (or an IDLE
  timeout) between pages.

Body downloads were already newest first (0036).

## Decision
Every provider syncs in two phases:

1. **Recent mail in every folder.** Messages from the last
   `initial_sync_days` (at least 7 for Gmail, JMAP and Outlook), plus flags
   and deletions.
2. **Older mail, newest first, in batches.** A budget per pass: 5000 IMAP
   headers, taken 250 at a time from each mailbox in turn; 1000 Gmail or
   JMAP messages; 25 Outlook pages of 200, five per folder in turn. Progress
   is kept in cursors, so it resumes after a restart.

When a pass leaves older mail to fetch, the provider calls
`SyncCtx::more_to_sync()`, and the account loop starts the next pass after
one second instead of waiting for changes. Each pass still pushes the user's
queued changes and picks up new mail first, so a long backfill never holds
those up.

Per provider:
- **IMAP:** `sync_mailbox` returns the missing old UIDs instead of fetching
  them, and the mailbox cursor keeps `backfill: true` until it has them all
  (so the "unchanged" shortcut doesn't skip it).
- **JMAP:** the full sync covers only the window (`after`), and deletes
  only window messages the server no longer lists. The `jmap:backfill`
  cursor is the date the next page ends `before`.
- **Outlook:** the delta asks for `$orderby=receivedDateTime desc`, the one
  order Graph supports for it, with no filter, since a filtered delta caps
  at 5000 messages. The delta's `nextLink` is saved after every page, so a
  round can stop once it reaches mail older than the window and continue
  later.
- **Gmail:** unchanged, apart from signalling `more_to_sync`.

## Consequences
- Recent mail across all folders is there within the first pass; a large
  archive fills in after it, roughly together across folders.
- Search and the AI folder are incomplete for older mail until the backfill
  finishes.
- A JMAP or Gmail resync after an expired state only reconciles deletions
  in the window. Older mail deleted while we were away can linger. This
  was already true for Gmail.
- IMAP servers without CONDSTORE refetch every mailbox's flags on each
  backfill pass; the budget per pass keeps that rare enough.
