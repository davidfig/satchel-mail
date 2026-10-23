# 0004. SQLite as the local store, all SQL in one module

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

The store must handle 100k+ messages per account, survive crashes, be shared by
the app and agent processes, and later be portable to Postgres.

## Decision

One SQLite database (WAL mode) per data directory with denormalized thread
summaries. All SQL is in `crates/core/src/store.rs` and `schema.sql`. Raw
messages and credentials are *not* in the database: blobs go to a
content-addressed `BlobStore` (0005), credentials to a `SecretStore` (0015);
both are traits.

## Consequences

- Porting to Postgres for M7 means re-implementing one module; extracting a
  `Store` trait is the first M7 task.
- Other processes see commits via `PRAGMA data_version`, which the engine
  polls to emit `DataChanged`.
- Never `VACUUM` without rebuilding the search index (0014).
