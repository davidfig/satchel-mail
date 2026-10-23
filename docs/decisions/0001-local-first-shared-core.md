# 0001. Local-first desktop app around a shared core

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Users want their mail on their own disk where AI tools can read it, plus a
normal mail client. A paid hosted version should follow later without a
rewrite.

## Decision

Build a desktop app (Tauri) whose logic lives entirely in a Rust library,
`satchel-core`. Hosts are thin: the desktop app, the `satchel` CLI
(`serve`, `mcp`, `sync`), and later a cloud server all wrap the same `Engine`.
Every client action is a `satchel_api::Command`; the core knows nothing about
Tauri.

## Consequences

- The hosted version (M7) can reuse the core and UI unchanged.
- Only one process may own sync for a data directory (`engine.lock`); others
  (e.g. `satchel mcp`) run in read + propose mode.
- Host-specific features (file save dialogs, opening links) stay in the host
  and never leak into the core.
