# Satchel Mail — notes for Claude

Local-first email + calendar client. A Rust core syncs accounts into SQLite;
the desktop app (Tauri) and `satchel serve` host it; AI agents read through MCP
or the `~/Satchel/` folder and can only **propose** changes. See `README.md`
for the layout and `docs/` for design and decisions.

## Ground rules

- **Dependencies:** don't add a crate or npm package without a good reason.
  Prefer the standard library or a dependency we already have. A new dependency
  is justified for wire formats and protocols (MIME, IMAP, SMTP, iCalendar
  recurrence, TLS, HTTP), security-sensitive code (HTML sanitizing, crypto,
  static file serving), or platform integration (keychain, Tauri) — not for
  small helpers we can write in a few dozen lines. Pin exact versions. When you
  add, remove or replace one, update `docs/decisions/0003-dependencies.md`.
- **Decisions:** any significant design or product decision gets a record in
  `docs/decisions/` (copy `template.md`, next number, add it to the index in
  `docs/decisions/README.md`). If a change overturns a decision, mark the old
  record "Superseded by NNNN" instead of editing history away.
- **Agents never act directly.** Agent callers may only read and create
  proposals (`Command::allowed_for_agent`). Never add an MCP tool or agent
  command that sends, deletes, or changes settings.
- **One command surface.** Everything the UI, CLI and MCP do goes through
  `satchel_api::Command` → `Engine::handle`. Don't add host-specific backdoors.
- **All SQL lives in `crates/core/src/store.rs`** (and `schema.sql`), so the
  store can be ported to Postgres for the hosted version.
- **UI is vanilla TypeScript** (no framework). Only `ui/src/transport.ts` may
  touch Tauri APIs.
- Email content is untrusted: sanitize HTML in the core, render it in the
  sandboxed iframe, never `innerHTML` untrusted text.

## Commands

```bash
scripts/dev-servers reset                 # local Stalwart (IMAP/SMTP/JMAP/CalDAV) + fixtures
cargo test --workspace                    # unit + engine integration tests
cargo clippy --workspace                  # keep at zero warnings
(cd ui && npx tsc --noEmit && npm test && npm run build)
export SATCHEL_DATA_DIR=$PWD/.dev/test-data SATCHEL_SECRETS=file
target/debug/satchel call '{"cmd":"list_threads","args":{"query":{"search":"from:bob"}}}'
target/debug/satchel serve --ui ui/dist   # web UI on http://127.0.0.1:7781
```

Test accounts: `alice|bob|carol@satchel.test`, password `test-pass-<name>`,
self-signed TLS (use `--insecure` / `allow_invalid_certs`). Never point tests
at the user's real data directory — set `SATCHEL_DATA_DIR`.

## Where things are

- `crates/api` — the command/event schema (mirror changes in `ui/src/types.ts`).
- `crates/core` — `engine.rs` (account loops, indexer, mirror), `commands.rs`
  (dispatch, policies, proposals), `ops.rs` (outbound queue), `store.rs`,
  `search.rs` (query parsing → FTS5), `mirror.rs`, `calendar.rs`.
- `crates/providers` — one file per protocol, implementing the traits in
  `core/src/provider.rs`.
- `crates/mcp` — MCP tools. `crates/cli` — `satchel` binary + HTTP server.
- `apps/desktop` — Tauri host. `ui/` — front end.

## Conventions

- Rust edition 2024, `anyhow` for errors; use `error::err(ErrorCode, …)` when
  the client needs a specific error code.
- Match the surrounding style: short doc comments on public items, comments
  only where the why isn't obvious.
- Commits: imperative subject, body explaining why; end with the
  `Co-Authored-By` line given in the session.
