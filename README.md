# Satchel Mail

A desktop email and calendar client that keeps your mail on your own disk and
lets AI agents help — safely. Agents read your mail and calendars through MCP
(or a plain-text folder) and **propose** replies, filing and events. Nothing is
sent, moved or scheduled until you approve it in AI Proposals.

- **Accounts:** Gmail + Google Calendar, Microsoft 365 / Outlook, any IMAP/SMTP
  server (+ CalDAV), Fastmail and other JMAP servers.
- **A real mail client:** unified inbox, threads, fast full-text search with
  operators, compose/reply/forward with undo-send, drafts sync, attachments,
  remote-image blocking, keyboard shortcuts, command palette, dark mode.
- **Calendar:** day/week/month/agenda, recurring events, invitations with RSVP
  from the email itself, free-time search.
- **Contacts:** built from the people you write to, editable and mergeable
  (one person, many addresses), synced with CardDAV, Google Contacts and
  Outlook contacts.
- **AI surface:** MCP server (`satchel mcp`) with 27 read + propose tools, an
  AI-readable folder (`~/Satchel/`), per-agent access policies, an audit log.

See [docs/DESIGN.md](docs/DESIGN.md) for the architecture and
[docs/decisions/](docs/decisions/README.md) for the decisions behind it.

## Layout

```
crates/api        command/event schema shared by every host and client
crates/core       store (SQLite + FTS5), sync engine, op queue, proposals, AI mirror
crates/mime       MIME parse/build, HTML sanitizing, quoting
crates/ical       iCalendar parse/serialize, recurrence expansion, iMIP
crates/providers  IMAP/SMTP, JMAP, Gmail, Google Calendar + Contacts, Microsoft Graph, CalDAV, CardDAV, OAuth
crates/mcp        MCP server (read + propose tools)
crates/cli        `satchel` binary: serve (web UI + API), mcp, sync, accounts, agents
apps/desktop      Tauri desktop app
ui/               vanilla TypeScript UI (shared by desktop and web)
scripts/          dev-servers (local Stalwart), seed fixtures, release build
```

## Quick start (development)

Requirements: Rust 1.90+, Node 20+, Python 3 (for fixtures).

`scripts/run` builds and starts everything (`scripts/run --help`); the same
commands are pnpm scripts in the root `package.json`:

```bash
pnpm dev              # web UI with hot reload on http://127.0.0.1:7780, test data
pnpm app              # desktop app on your real data (OAuth client ids from .env)
pnpm web              # web UI on http://127.0.0.1:7781 (open the link it prints), real data
pnpm web:test         # web UI against the local test servers and .dev/test-data
pnpm servers:reset    # restore and reseed the local test servers
```

On macOS, create an Apple Development certificate once (Xcode → Settings →
Accounts → Manage Certificates → + → Apple Development; a free personal team
works). `scripts/run` signs dev builds with it, so the keychain's "Always
Allow" still holds after a rebuild instead of prompting every time. The
keychain ties each item to the signer's Team ID, so the self-signed
certificate from `scripts/dev-signing-cert` is only a fallback: with it macOS
still asks once per item after each rebuild.

The steps by hand:

```bash
scripts/dev-servers reset        # install + start local Stalwart, seed test mail and events
(cd ui && npm install && npm run build)
cargo build -p satchel-cli
```

Run the web UI against a throwaway data directory:

```bash
export SATCHEL_DATA_DIR=$PWD/.dev/test-data SATCHEL_SECRETS=file
SATCHEL_PASSWORD=test-pass-alice target/debug/satchel accounts add-imap \
  --email alice@satchel.test --name Alice --username alice \
  --imap-host 127.0.0.1 --imap-port 2993 --smtp-port 2465 \
  --caldav-url http://127.0.0.1:8480/dav/cal --insecure
target/debug/satchel serve --ui ui/dist      # open the http://127.0.0.1:7781/#session=… link it prints
```

Run the desktop app in development:

```bash
cp target/debug/satchel apps/desktop/binaries/satchel-$(rustc -vV | sed -n 's/host: //p')
(cd apps/desktop && ../../ui/node_modules/.bin/tauri dev)
```

More in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Connecting an AI agent

Settings → AI agents → **Connect an agent** creates a token and shows the exact
command, e.g. for Claude Code:

```bash
claude mcp add satchel --env SATCHEL_AGENT_TOKEN=stk_… -- satchel mcp
```

Details, the tool list, and the folder-based alternative are in
[docs/AI_AGENTS.md](docs/AI_AGENTS.md).

## Releases

```bash
scripts/release.sh                # CLI + desktop bundles for this platform
```

Google and Microsoft sign-in need OAuth client ids — see
[docs/OAUTH.md](docs/OAUTH.md). Signing, notarization and the auto-updater are
configured through environment variables documented at the top of
`scripts/release.sh`.

## Tests

```bash
cargo test --workspace
(cd ui && npx tsc --noEmit)
```
