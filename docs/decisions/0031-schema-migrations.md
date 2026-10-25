# 0031. Versioned schema with ordered migrations

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

`schema.sql` was run on every open as `CREATE ... IF NOT EXISTS`. That adds
new tables but never changes existing ones, so the first new column on an
existing table would have left real databases in the old shape, failing with
"no such column".

## Decision

The schema version lives in `PRAGMA user_version`. `schema.sql` always
describes the current schema, `SCHEMA_VERSION`. `MIGRATIONS[i]` takes a
version `i + 1` database to `i + 2`. Each migration runs in a transaction
with its version bump (`store/mod.rs`, `migrate`).

- A new database gets `schema.sql` and the current version.
- A database from before versioning (`user_version` 0 with tables) is
  version 1. The schema had only gained tables until then, which
  `schema.sql` adds.
- A database from a newer Satchel is refused, not opened.

To change the schema, edit `schema.sql`, bump `SCHEMA_VERSION`, and append
the migration. Migrations are SQL only, so they port to Postgres with the
rest of the store (0004, 0018).

## Consequences

- Schema changes are now deliberate: each needs a migration and a test that
  opens an old database.
- Going back to an older build after a migration isn't supported.
