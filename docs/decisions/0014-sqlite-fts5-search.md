# 0014. Full-text search with SQLite FTS5

- **Status:** Accepted (supersedes the tantivy index)
- **Date:** 2026-10-23

## Context

Search first used a separate tantivy index: it returned the top 10,000 message
ids, which SQL then filtered and paged. Results beyond that cap were lost,
counts were wrong for broad queries, and mail without a downloaded body
wasn't searchable.

## Decision

Use a contentless FTS5 table (`message_fts`, keyed by `messages.rowid`) in the
same database. Free text becomes a safe MATCH expression (`search::fts_query`:
prefix terms, quoted phrases, everything else stripped) and joins the same
WHERE clause as every other filter, so text, operators, date order, counts and
paging are one query with no cap. Headers are indexed as soon as a message
syncs; the entry is rebuilt with the body once it downloads. A delete trigger
keeps the index in step.

## Consequences

- One store, no separate index directory, consistent across processes; maps to
  Postgres full-text search for M7.
- Removes the tantivy dependency.
- Rowids must stay stable: rebuild the index (`Store::reset_indexed`) after any
  VACUUM.
