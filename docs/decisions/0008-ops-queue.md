# 0008. Optimistic local changes with a durable op queue

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

The UI must feel instant and work offline, but providers are slow and fail.

## Decision

User actions change the store immediately and enqueue an op (`ops` table). Each
account loop pushes ops before syncing, in order, with exponential backoff
(sends don't block other ops). A permanent failure is reported, audited, and
triggers a full resync so local state converges back to the server. Ops that
move messages snapshot their source location at enqueue time.

## Consequences

- A brief flicker is possible if a sync lands before a retried op succeeds.
- Undo-send is a 5-second client-side delay before the send op is queued.
