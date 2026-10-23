# 0002. Rust core and a vanilla TypeScript UI

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

The core must be fast, memory-efficient and safe with untrusted input; the UI
must run in Tauri's webview and in a browser for the hosted version.

## Decision

Rust for the core and protocol adapters. The UI is plain TypeScript built with
Vite: small DOM helpers (`ui/src/lib/dom.ts`), no framework. `ui/src/transport.ts`
is the only module allowed to touch Tauri; it also implements HTTP + WebSocket
for the web build.

## Consequences

- Tiny bundle (~80 KB JS) and no framework upgrade churn.
- More hand-written UI code (virtual list, modals, menus).
- The same UI ships in the desktop app and `satchel serve`.
