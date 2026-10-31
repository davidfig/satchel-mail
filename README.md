# satchel.email

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

## Status: not released yet

The code is open and builds today ([Quick start](#quick-start-development)),
but there are no downloads yet. The first release, v0.1.0, is waiting on:

1. **Google's review of Gmail access.** `https://mail.google.com/` is a
   restricted scope, so until Google verifies the app only listed test users
   can sign in with Google. Verification needs the satchel.email site live
   (home page and privacy policy, on a verified domain), a demo video of the
   sign-in and of how each scope is used, and a yearly independent security
   assessment (CASA). See [docs/OAUTH.md](docs/OAUTH.md).
2. **Signed installers:** an Apple Developer ID with notarization for Mac, and
   Azure Artifact Signing for Windows, set up as described in
   [docs/RELEASING.md](docs/RELEASING.md).
3. **Microsoft publisher verification,** so the Microsoft 365 and Outlook
   consent screen doesn't say "unverified". Sign-in works without it.

Until then, Fastmail, iCloud and any IMAP or JMAP server work fully. Gmail
works as an IMAP/SMTP account with a
[Google app password](https://myaccount.google.com/apppasswords) (mail only;
Google Calendar and Contacts need the Google sign-in). To build with your own
Google or Microsoft sign-in, create your own OAuth client ids
([docs/OAUTH.md](docs/OAUTH.md)).

## License

| Part | License |
| --- | --- |
| The app: `core`, `providers`, `mcp`, `cli`, `apps/desktop`, `ui`, and everything else not listed below | [AGPL-3.0](LICENSE) |
| The libraries: `crates/mime`, `crates/ical`, `crates/api` | MIT OR Apache-2.0 (each crate's `LICENSE-MIT` and `LICENSE-APACHE`) |

The satchel.email name and logo aren't covered by these licenses: see
[TRADEMARKS.md](TRADEMARKS.md). Dependencies keep their own licenses: see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). To contribute, see
[CONTRIBUTING.md](CONTRIBUTING.md). Why: [0058](docs/decisions/0058-licensing.md).

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
site/             marketing site (static HTML, no build)
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

## Installing on your Mac

```bash
npm run install-app               # release build → /Applications/satchel.email.app
```

Runs it like a normal app when you're not developing; run it again to update.
It compiles in the OAuth ids from `.env`, signs with your Apple Development
certificate, and quits a running copy first. The installed app and
`scripts/run` share the default data directory, so only one can run at a time.
On another computer, copy `.env` over first (it isn't in git).

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
