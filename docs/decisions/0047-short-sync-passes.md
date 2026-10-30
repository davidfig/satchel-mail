# 0047. Short sync passes while older mail fills in

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
0039 made every provider sync recent mail first and older mail in batches,
but both ran inside one `MailProvider::sync` call, and each pass then
downloaded up to 300 message bodies before the calendar and contacts. The
account only counted as synced, and only looked for new mail again, once
all of that was done. On a new computer with several large accounts a pass
took 15 minutes or more, so new mail waited that long and the sidebar
showed "16m" next to accounts that were busy syncing.

Outlook had a second gap: a folder's delta round only reports mail that
arrived after the round began when the round ends, so a folder with years
of older mail to fetch hid new mail until its backfill finished.

## Decision
- `MailProvider::sync` covers recent mail only. Older mail moves to a new
  `MailProvider::backfill(ctx, until)`, which fetches newest first until a
  deadline and calls `more_to_sync` if some is left. Providers keep what
  `sync` found (IMAP's missing UIDs, Outlook's unfinished folders, Gmail's
  labels) for the `backfill` that follows in the same pass.
- The account loop stamps the account synced right after `sync`, then
  downloads bodies for at most 30 seconds and older mail for at most 60,
  and starts the next pass a second later while older mail is left.
  Batch sizes per pass (5000 IMAP headers, 1000 Gmail or JMAP messages,
  25 Outlook pages) give way to these times; Gmail and JMAP pages are now
  500 messages so the deadline is checked more often.
- Outlook folders still in a delta round also get their newest 50
  messages from a plain listing on every pass.

## Consequences
- New mail shows up within about a minute and a half even during a long
  first sync, and "last synced" stays current.
- Passes repeat more often during a backfill, so the recent check (folder
  list, a SELECT per IMAP mailbox) runs more often. IMAP servers without
  CONDSTORE refetch flags each pass, as 0039 noted, now about once a
  minute and a half while older mail is coming in.
- Body downloads per pass are bounded by time as well as count, so a slow
  server downloads fewer per pass.
- Updates 0039, whose two-phase order stands.
