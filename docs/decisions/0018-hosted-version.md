# 0018. Hosted (cloud) version

- **Status:** Proposed — on hold
- **Date:** 2026-10-23

## Context

A paid hosted tier would offer always-on sync and remote agent access.

## Decision

Not decided. Planned shape: `apps/server` (axum) hosting the same core and UI,
Postgres + object storage + KMS behind the store, blob and secret seams,
per-account sync workers with provider push, MCP over HTTP with OAuth.
Discuss before building.

## Consequences

- Prerequisite: extract a `Store` trait (0004).
