# 0037. The user's requests go before background sync

- **Status:** Accepted
- **Date:** 2026-10-28

## Context
Opening a message whose content isn't stored yet, or downloading an
attachment, fetches from the server while the user waits (30 s and 120 s
timeouts). Background sync talks to the same server at the same time: Gmail
and Graph sync run several requests at once, and the body downloader works
through hundreds of messages. On a busy pass the user's request queued behind
sync's, and when sync had used up the provider's rate limit, the user's
request sat out the same backoff (up to 60 s per retry) and timed out.

The user's changes (read, archive, send) also waited: the account loop pushed
queued ops only at the start of a pass, so a change made mid-pass waited for
the whole sync and body download to finish.

## Decision
- Each account has a priority gate counting user requests in flight. A user
  fetch runs in the *user lane*; sync, body downloads, calendar and contacts
  sync run in the *background lane*. The lane is a tokio task-local, so
  providers need no new arguments (`provider::priority`).
- Background work calls `yield_to_user()` before each request (the HTTP
  `ApiClient`, JMAP calls, IMAP batches and folders, each body download) and
  waits while a user request is in flight, at most 60 s at a time.
- In the user lane, rate-limit backoff is capped at 4 s per retry rather than
  honouring a long Retry-After; with background paused, the quota recovers
  quickly.
- The account loop pushes due ops after mail sync, before calendar and
  contacts sync, and between body downloads, not only at the start of a pass.

## Consequences
- Opening a message or attachment no longer competes with sync, and rarely
  times out because of it. Requests already in flight finish; only the next
  ones wait.
- Sync is slower while the user is fetching, by design.
- IMAP: a user fetch while sync holds the cached connection opens a second
  connection, as before; the yields keep that window short.
- Work spawned onto another task loses its lane (and so its priority); keep
  provider requests on the calling task.
