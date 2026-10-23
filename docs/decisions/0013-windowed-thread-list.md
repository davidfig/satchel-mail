# 0013. Windowed thread list and stable paging

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Joint inboxes can hold hundreds of thousands of threads. The first version
paged by date alone (skipping threads with equal timestamps) and refreshed by
reloading up to 500 threads.

## Decision

The list is a windowed cache addressed by offset and sized by `CountThreads`:
at most 30 pages of 100 threads, placeholders for rows still loading, and a
scrollbar compressed above 8M px so million-thread folders scroll. A refresh
re-reads only the visible window and keeps the top visible thread anchored.
API and MCP paging is keyset by `(last_date, thread id)`. Mailbox filters pick
an index-driven or scan-driven query plan by mailbox size.

## Consequences

- 100k threads: pages ≈2 ms, deep offsets ≤90 ms, counts ≈65 ms.
- Offset pages can briefly overlap if mail arrives between page loads; the next
  refresh corrects it.
