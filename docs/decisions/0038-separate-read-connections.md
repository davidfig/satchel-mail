# 0038. Background reads use their own connection

- **Status:** Accepted
- **Date:** 2026-10-29

## Context
The store had one write connection and one read connection, each behind a
mutex. Every read shared that one connection: the user's requests and the
background workers (AI folder mirror, search indexer, contacts pass). A slow
background read made every UI request wait. During sync the mirror rewrote
the folder indexes after each batch, and one of those queries took ~5 s on a
real account, so switching messages stalled for seconds at a time. The
data-version watcher, an async task, also waited on that mutex and blocked a
Tokio worker thread while it did.

## Decision
- The store opens three read connections: `read` for the user's requests,
  `bg_read` for background workers, and `watch` used only by
  `PRAGMA data_version`.
- Workers run their blocking passes inside `store::background(|| …)`, which
  sets a thread-local so `Store::r` picks `bg_read`. Callers need no new
  arguments, and the store API is unchanged.
- Writes still go through the one write connection. In WAL mode readers don't
  block the writer or each other.

## Consequences
- A slow background query no longer delays opening a message or refreshing
  the list. It still holds up other background work.
- The flag is per thread: only blocking work on one thread (`spawn_blocking`)
  can use it. Async work that moves between Tokio threads stays on the
  user's connection. That includes the account loop's sync, body downloads
  and `ingest_dropped_proposals`.
- Background writes still queue behind other writes on the write connection.
  Keep write transactions short.
- A hosted Postgres store would use a connection pool and do the same split
  there.
