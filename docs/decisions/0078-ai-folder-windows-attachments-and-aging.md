# 0078. The AI folder keeps to its date limits over time, and holds the only copy of recent attachments

- **Status:** Accepted
- **Date:** 2026-11-10
- **Amends:** 0036 (attachments no longer keep stored copies whole), 0074

## Context

Two gaps in the AI folder (0022), found while checking it against a real
mailbox:

- **Nothing removed mail that aged past the date limit.** A thread folder
  was only rewritten when its mail changed, so a conversation stayed long
  after it was older than `since_days`. On a real mailbox, 438 of 12,275
  thread folders were entirely older than the one-year limit.
- **Attachments were all or nothing, and stored twice.** Turning on
  "Include attachments" made every message download whole, back to the
  oldest, and kept each attachment both base64-encoded in the stored copy
  and decoded in the folder. Messages already stored without their
  attachments (most big ones) never got them in the folder.

Agents mostly need attachments of recent mail: the receipt, the contract,
this month's slides. On the same mailbox, the last 90 days hold about 620
attachments (around 100 MB).

## Decision

- **A daily sweep** (with the check for missing folders, at startup and
  then once a day) queues every written thread with a message older than
  the date limit to be written again, which drops those messages or
  removes the folder. A thread that straddles the limit is rewritten each
  day; on the real mailbox that's 4 threads.
- **An attachment window**, `attachment_days`, 90 by default, inside the
  text window. "Include attachments" stays off by default.
- **Attachments are downloaded for the folder alone.** Stored copies stay
  slim (0036). A background worker takes messages inside the window whose
  thread is written, newest first, gives way to the user like other
  background downloads (0037), fetches the full message and writes only
  its attachments to `attachments/<n>-<name>` in the thread's folder (up to
  25 MB each). If the stored copy is already whole (the user opened an
  attachment), the thread writer uses it instead. Satchel never reads the
  folder's copies back, so deleting one loses nothing the server doesn't
  have; opening an attachment in the app still downloads it (0036).
- **`messages.ai_attachments`** (schema 11) records which messages' files
  are written. A rewrite keeps those files; if one is missing, the flag is
  cleared and the worker fetches it again. The daily sweep clears the flag
  for messages past the window (all of them, when attachments are turned
  off) and rewrites their threads, which removes the files.
- "What counts as an attachment" is the rule of 0077: not images shown in
  place, nor Gmail's AMP body.
- `sync.json` gains `attachment_days` and `pending_attachments` while
  attachments are on, and the README explains both.

## Consequences

- The folder stays within its limits without a settings change, and holds
  each recent attachment once.
- Turning attachments on costs one download per message in the window
  (about 620 on the real mailbox), then a few a day.
- A file's name depends on its message's position in the thread. When an
  older message ages out of the thread, the names shift and the files are
  downloaded again. This is rare (the attachment window is inside the text
  window) and keeps the code simple.
- A download that fails is skipped until the app restarts.
- `.eml` files still keep stored copies whole, as before.
