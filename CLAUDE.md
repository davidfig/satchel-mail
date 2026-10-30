# Satchel Mail — notes for Claude

Local-first email + calendar client. A Rust core syncs accounts into SQLite;
the desktop app (Tauri) and `satchel serve` host it; AI agents work through the
`~/Satchel/` folder (or MCP) and can only **propose** changes. See `README.md`
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
  proposals (`Command::allowed_for_agent`); the user's per-agent policy decides
  whether a proposal applies on arrival or waits in Review (0023). Never add
  an MCP tool or agent command that sends, deletes, or changes settings, and
  never let agents read-write their own policy.
- **Files first for agents.** The AI folder (`~/Satchel/`, `core/src/ai_folder/`) is the
  main agent interface; MCP is the second door (0022). New agent features
  should work through the folder, not only through MCP.
- **One command surface.** Everything the UI, CLI and MCP do goes through
  `satchel_api::Command` → `Engine::handle`. Don't add host-specific backdoors.
- **All SQL lives in `crates/core/src/store/`** (one file per table group,
  plus `schema.sql`), so the store can be ported to Postgres for the hosted
  version. Schema changes need a migration (0031).
- **UI is vanilla TypeScript** (no framework). Only `ui/src/transport.ts` may
  touch Tauri APIs.
- **Reuse UI components; don't rebuild them.** Shared pieces live in
  `ui/src/lib/` (`dom.ts` helpers and icons, `ui.ts` toasts/modals/menus/
  shortcuts, `tooltip.ts`) and are styled once in `ui/src/styles/` with the
  tokens. Use them instead of one-off versions, and when a view needs
  something new that another view could use, add it to `lib/` first. Never
  use native browser UI where we have our own: tooltips come from
  `title: '…'` in `h()` (rendered by `tooltip.ts`, never the browser's), and
  every button has one, with its shortcut in parentheses if it has one. See
  `docs/decisions/0021-shared-ui-components.md`.
- **Redraw with `patch`, not `replace`,** anywhere a view redraws while it's
  on screen (sync ticks, events, typing), or it blinks. Listeners go through
  `on`/`listen`, and handlers read `e.currentTarget`, not a captured element
  (0046).
- Email content is untrusted: sanitize HTML in the core, render it in the
  sandboxed iframe, never `innerHTML` untrusted text.

## Commands

```bash
scripts/dev-servers reset                 # local Stalwart (IMAP/SMTP/JMAP/CalDAV) + fixtures
scripts/run [web] [--test]                # build + run the app (also `pnpm app|web|web:test`)
scripts/dev-sign --sign-only BIN          # sign with the Apple Development cert so keychain "Always Allow" sticks
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

Code is grouped in folders by domain, files stay under ~400 lines, and a
folder's `mod.rs` (or `index.ts`) says what's in it (0035).

- `crates/api` — the command/event schema, one file per domain (`mail.rs`,
  `calendar.rs`, `command.rs`, …). Mirror changes in the matching file in
  `ui/src/types/`.
- `crates/core/src`:
  - `dispatch/` — `Command` → handler routing, and agent access checks.
  - `mail/` (read, write, drafts and sending, folders, search, spam),
    `calendar/` (expansion, event writes, emailed invites), `contacts/`
    (automatic contacts, merging, server cards, commands), `agents/`
    (identity, proposals), `accounts.rs`.
  - `engine/` — the `Engine`, account loops, background workers.
  - `ops/` — the outbound op queue, one file per kind of op.
  - `store/` — all SQL, one file per table group; `schema.sql`.
  - `ai_folder/` — `export/` writes `~/Satchel`, `import/` reads proposal and
    person files, `safe_fs.rs` refuses symlinks.
  - `provider/` — the traits providers implement, and `SyncCtx`.
- `crates/providers/src` — by vendor: `imap/`, `jmap/`, `google/` (Gmail,
  Calendar, Contacts), `microsoft/` (mail, calendar, contacts), `dav/`
  (CalDAV, CardDAV), `places/` (Photon, Apple Maps); `net.rs`, `oauth.rs`.
- `crates/ical`, `crates/mime` — parsing and building, split by concern.
- `crates/mcp` — MCP tools. `crates/cli` — `satchel` binary + HTTP server.
- `crates/core/tests` — integration tests by area, with fakes in `support/`.
- `apps/desktop` — Tauri host.
- `ui/src` — `app/` (shell, palette), `views/mail/`, `views/settings/`,
  `views/*.ts`, `lib/` (shared components), `types/`, `styles/`.

## Conventions

- Rust edition 2024, `anyhow` for errors; use `error::err(ErrorCode, …)` when
  the client needs a specific error code.
- Match the surrounding style: short doc comments on public items, comments
  only where the why isn't obvious.
- Commits: imperative subject, body explaining why; end with the
  `Co-Authored-By` line given in the session.
