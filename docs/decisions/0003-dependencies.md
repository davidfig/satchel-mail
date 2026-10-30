# 0003. Build from scratch; depend only where it pays

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

We want to own the product (sync, store, approval model, UI) and keep the
supply chain small, but hand-writing wire-format parsers is a large bug
surface with no product upside.

## Decision

Write the application ourselves. Take a dependency only for wire formats and
protocols, security-sensitive code, or platform integration, and pin exact
versions. Small helpers are written in-tree (e.g. SHA-256 for PKCE,
percent-encoding, random tokens from UUID v4).

Current third-party dependencies and why:

| Crate / package | Why |
|---|---|
| tokio, async-trait, futures | async runtime; object-safe async provider traits; concurrent request streams |
| serde, serde_json, chrono, chrono-tz, uuid, base64, anyhow, tracing | basic data, time zones, ids/randomness, encoding, errors, logs |
| rusqlite (bundled SQLite) | the store and full-text search (FTS5) |
| blake3 | fast content addressing of message blobs |
| directories | platform data-directory conventions |
| keyring | OS keychain |
| mail-parser, mail-builder | RFC 5322 / MIME |
| ammonia | HTML sanitizing (security-critical) |
| html5ever | HTML tokenizing for our HTML → text conversion (0043). Already in the tree through ammonia, at the same version, so it adds no new code |
| rrule | RFC 5545 recurrence expansion |
| async-imap | IMAP protocol grammar (we own the sync logic) |
| lettre | SMTP submission |
| reqwest, rustls, tokio-rustls, rustls-native-certs, url | HTTP/TLS for JMAP, CalDAV, Gmail, Graph, OAuth, and place lookups (0050) |
| quick-xml | WebDAV/CalDAV XML |
| libc (unix core, macOS desktop) | `openat`/`renameat`/`unlinkat` so the AI folder can't be tricked by symlinks swapped in mid-walk, and `setxattr` to quarantine saved attachments (0040) |
| windows-sys (Windows core; `Win32_Foundation`, `Win32_Storage_FileSystem` only) | the Windows side of the same AI-folder safety: link counts, a handle's final path, and renaming and deleting by handle, none of which stable std exposes. Already in the tree through tokio and others, so it adds no new code |
| objc2, objc2-foundation, objc2-map-kit (macOS only; `MKLocalSearchCompleter` and the Foundation types it uses) | Apple Maps place suggestions in the Mac app (0050). objc2 and objc2-foundation are already in the tree through Tauri, at the same versions; objc2-map-kit is the generated MapKit binding from the same project |
| rmcp | MCP protocol |
| axum, tower-http, clap, tracing-subscriber | local HTTP server + static files, CLI parsing, logging |
| tauri + plugins (opener, dialog, notification, single-instance, window-state, updater) | desktop shell; notification shows new-mail alerts in the macOS, Windows and Linux notification centers (0025); window-state reopens the window at its last position and size, including which monitor and whether it was maximized |
| @tauri-apps/api; dev: vite, typescript, @tauri-apps/cli, linkedom | UI ↔ Tauri bridge; build tools; linkedom gives `node --test` a DOM for the UI's quote-detection tests (small, no native code; a test-only stand-in for the browser's DOMParser) |

Removed: tantivy (see 0014), rand, urlencoding, bytes, thiserror.

## Consequences

- Every new dependency needs a row here with its reason.
- We accept writing and testing more code ourselves in exchange for a smaller,
  auditable dependency tree.
