# Development

## Test servers

`scripts/dev-servers` runs [Stalwart](https://stalw.art) natively (no Docker)
on localhost-only ports. One server covers every protocol we test:

| Protocol | Port |
|---|---|
| IMAP (STARTTLS) / IMAPS | 2143 / 2993 |
| SMTP submission (STARTTLS) / submissions | 2587 / 2465 |
| SMTP inbound (delivery) | 2525 |
| HTTP: JMAP (`/jmap/session`), CalDAV (`/dav/cal`), CardDAV (`/dav/card`), admin | 8480 |

Accounts: `alice`, `bob`, `carol` @`satchel.test`, password `test-pass-<name>`.
The server uses a self-signed certificate, so add accounts with
`allow_invalid_certs` (`--insecure` in the CLI, a checkbox in the UI).

```bash
scripts/dev-servers install   # download stalwart + stalwart-cli to .dev/bin
scripts/dev-servers init      # first-time setup (ports, accounts) + save a template
scripts/dev-servers start | stop | status
scripts/dev-servers reset     # restore the clean template, start, seed fixtures
scripts/dev-servers seed [--user bob] [--bulk 20000]
```

Contacts sync against the server is covered by an ignored end-to-end test:
`cargo test -p satchel-providers --test carddav_live -- --ignored` (needs the
servers running).

Fixtures (`scripts/seed.py`) include a four-message thread, HTML with remote
images and a script tag, attachments, an inline image, non-ASCII headers, an
iMIP invitation, a prompt-injection attempt, and a recurring calendar event.

## Running

Everything the UI and agents do goes through `satchel_api::Command`. The CLI
can run any command directly, which is the quickest way to test the core:

```bash
export SATCHEL_DATA_DIR=$PWD/.dev/test-data SATCHEL_SECRETS=file
target/debug/satchel call '{"cmd":"list_threads","args":{"query":{"search":"from:bob is:unread"}}}'
target/debug/satchel sync               # one sync pass for every account, then exit
target/debug/satchel serve --ui ui/dist # web UI + API; open the ?session= link it prints
```

Only one process may own sync for a data directory (`engine.lock`). Other
processes — `satchel mcp`, `satchel call` while the app is running — open the
store read-only plus proposals.

For UI work with hot reload, run `satchel serve` and `npm run dev` in `ui/`
with `SATCHEL_SESSION_TOKEN` set to the same value in both shells; Vite proxies
`/api` to the server.

## Secrets

The desktop app and CLI store credentials in the OS keychain
(`app.satchel.mail`). `SATCHEL_SECRETS=file` uses `<data dir>/secrets.json`
(mode 0600) instead — for tests, CI and headless servers.

## Data directory

```
satchel.db      SQLite (WAL): accounts, mailboxes, messages, threads, events, ops, proposals, audit,
                and the FTS5 full-text index
blobs/          raw RFC 822 messages, content-addressed (BLAKE3)
engine.lock     held by the process that owns sync
```

Default location: `~/Library/Application Support/app.Satchel.Satchel-Mail`
(macOS), `%APPDATA%\Satchel\Satchel Mail\data` (Windows), `~/.local/share/satchelmail`
(Linux), or `$SATCHEL_DATA_DIR`.

## Decisions

Design decisions are recorded one per file in [decisions/](decisions/README.md). Add one for any significant change.

## Code map

- **Sync** — `core/src/engine/account_loop.rs` runs one loop per account: push queued ops,
  sync mail (provider delta), fetch recent bodies, sync calendars, then wait for
  push (IMAP IDLE, JMAP EventSource) or the poll interval.
- **Ops** — `core/src/ops/`. User actions change the store optimistically and
  enqueue an op; ops retry with backoff, and a permanent failure resyncs the
  kind of data it touched so local state converges back to the server (0030).
- **Threading** — `core/src/store/messages.rs::assign_thread`: provider thread ids when
  available, otherwise `References`/`In-Reply-To` in both directions, merging
  threads when a parent arrives late.
- **Proposals** — `core/src/agents/proposals.rs`: validation against the agent's policy,
  risk warnings, de-duplication, staleness checks on approval.
- **Providers** — `providers/src/<vendor>/`, each implementing `MailProvider`
  and/or `CalendarProvider` from `core/src/provider/`, and `ContactsProvider`
  (CardDAV, Google People, Graph contacts).
- **Contacts** — `core/src/contacts/`: automatic contacts from mail, edits,
  merges, and merging server changes with local edits (decision 0026).
