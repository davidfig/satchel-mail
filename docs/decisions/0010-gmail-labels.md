# 0010. Gmail labels as mailboxes, with a virtual All Mail

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Gmail messages carry many labels; archiving removes INBOX rather than moving.

## Decision

Each visible label is a mailbox with `is_label = true`; every message outside
Trash/Spam is also in a virtual "All Mail" mailbox. Archive removes the INBOX
label; Move adds the target and removes INBOX. Deltas come from
`history.list`; mail older than the initial window backfills one page per sync.
Sends include the Gmail thread id so replies stay threaded.

## Consequences

- Unread and starred are labels on the server but flags locally.
- Not yet verified against a live account (needs OAuth client ids, 0017).
