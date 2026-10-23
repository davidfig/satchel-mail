# Satchel Mail — Design & Build Plan

> Draft v0.2 — 2026-10-23. Name cleared in a basic USPTO search (no SATCHEL MAIL mark; no live SATCHEL software marks); get a formal clearance search before filing.

## 1. What we're building

A desktop email + calendar client that:

1. **Syncs** every account (Gmail, Microsoft 365, generic IMAP/CalDAV, Fastmail/JMAP) to a local store.
2. **Exposes** that store to external AI agents (Claude Code, Claude Desktop, any MCP client) in an AI-friendly form — readable files on disk plus an MCP server.
3. **Lets AI propose, never act**: agents can draft replies, propose labels/moves/archives, and propose calendar changes. Every proposal lands in a **review queue**; nothing reaches a server until a human approves it.
4. **Is a full modern mail client** on its own — unified inbox, threads, fast search, compose, calendar views — so it's usable with or without an AI attached.

Secondary: a **paid hosted version** that runs the same core in the cloud (always-on sync, AI access from anywhere). Not built in v1, but every v1 decision must keep it cheap to add (§9).

### Non-goals (v1)
- A built-in AI/LLM. Satchel Mail is the *substrate*; the AI is external. (A bundled local-model option can come later.)
- Contacts sync (CardDAV/People API) beyond what's needed for address autocomplete.
- Mobile apps.
- Auto-send rules for AI. Approval is mandatory in v1; policy-based auto-approve is a later feature.

## 2. Build vs. reuse

We write the application from scratch — sync engine, store, index, approval system, MCP server, UI. We do **not** wrap CLI tools (mbsync, notmuch, vdirsyncer, himalaya).

We **do** use libraries for wire formats and protocols, because hand-writing them is a large bug surface with no product upside:

| Concern | Crate | Why not write it |
|---|---|---|
| MIME / RFC 5322 parsing | `mail-parser` (Stalwart) | Decades of malformed real-world mail; charset hell |
| Message building | `mail-builder` | Correct encoding, folding, multipart |
| IMAP client | `async-imap` (or `imap-codec` + own client) | Protocol grammar; we still own the sync logic |
| SMTP submission | `lettre` | Auth mechanisms, STARTTLS |
| iCalendar parse/build | `calcard` (Stalwart) or `icalendar` | RRULE/timezones are notoriously hard |
| Full-text search | SQLite FTS5 (in `rusqlite`) | Same store and query as every filter; no extra dependency (see decisions/0014) |
| Storage | `rusqlite` (SQLite, WAL, FTS fallback) | — |
| HTML sanitizing | `ammonia` | Security-critical |
| OAuth 2 | `oauth2` | PKCE, refresh flows |
| HTTP / async | `reqwest`, `tokio`, `axum` | — |
| MCP | `rmcp` (official Rust SDK) | Spec tracking |
| Desktop shell | Tauri 2 | — |

Gmail API, Microsoft Graph, JMAP and CalDAV clients are thin REST/XML layers — we write those ourselves.

## 3. Architecture

```
┌────────────────────────── Desktop app (Tauri) ──────────────────────────┐
│                                                                         │
│   UI (vanilla TS, Vite)  ◄──── Transport (Tauri IPC | HTTP+WS) ────┐    │
│                                                                    │    │
│  ┌──────────────────────────── satchel-core (Rust) ──────────────┐ │    │
│  │                                                               │ │    │
│  │  API layer (typed commands + event stream) ◄──────────────────┘ │    │
│  │      │                                                          │    │
│  │  Ops log & Proposal queue ──► Mutation executor ──┐             │    │
│  │      │                                            ▼             │    │
│  │  Store (SQLite + blob dir) ◄── Sync engine ◄── Provider adapters│    │
│  │      │                                  (Gmail, Graph, IMAP,    │    │
│  │  Full-text search (SQLite FTS5)          JMAP, CalDAV, GCal)    │    │
│  │      │                                                          │    │
│  │  AI mirror writer (files on disk)                               │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                         │
│   MCP server (stdio + localhost HTTP) ── same API layer, AI scope ──────┤
└─────────────────────────────────────────────────────────────────────────┘
```

Key rule: **the core is a library with no knowledge of Tauri.** The desktop app and the future cloud server are two thin hosts around the same core.

### 3.1 Cargo workspace

```
satchel-mail/
  crates/
    core/            # domain model, store, sync engine, ops/proposals, search
    providers/       # one module per provider; implements core traits
    mime/            # wrappers around mail-parser/builder, HTML sanitize, quoting
    ical/            # wrappers around iCal parsing, recurrence expansion
    api/             # command/event schema (serde), shared by all hosts
    mcp/             # MCP server over the api crate, AI-scoped
    cli/             # `satchel` CLI: headless sync, debug, also usable by agents
  apps/
    desktop/         # Tauri shell + vanilla TS UI
    server/          # (later) axum host for the cloud version
  ui/                # vanilla TS frontend, shared by desktop and cloud web
  docs/
```

## 4. Data model

### 4.1 Storage layout (per profile)

```
~/Library/Application Support/Satchel Mail/  (platform equivalent elsewhere)
  satchel.db             # SQLite: accounts, folders, messages, threads, events, ops, proposals
  blobs/ab/cdef…eml      # raw RFC 822 messages, content-addressed (BLAKE3)
  attachments/…          # extracted on demand, content-addressed
~/Satchel/                                   (user-visible AI mirror, §6.1)
```

SQLite is the source of truth for metadata; raw messages live as immutable blobs. Both are behind a `Store` trait and a `BlobStore` trait so the cloud host can swap in Postgres + S3 (§9).

### 4.2 Core entities

- **Account** — provider kind, credentials ref (keychain id), capabilities.
- **Mailbox** — folder or label. Gmail labels and IMAP folders are both modeled as many-to-many `message ↔ mailbox` membership; folder-only providers simply have exactly one.
- **Message** — stable local id, provider id(s), `Message-ID`, thread id, headers summary, flags, blob hash, snippet.
- **Thread** — provider thread id where available (Gmail, Graph conversationId, JMAP threadId); otherwise our own JWZ-style threading on `References`/`In-Reply-To`.
- **Calendar**, **Event** — iCal-native (UID, RRULE, exceptions), with provider ids and etags.
- **Op** — an intended mutation (see §5).
- **Proposal** — a group of ops from an AI agent awaiting review.
- **SyncState** — per account/mailbox/calendar cursor (Gmail historyId, Graph deltaLink, IMAP UIDVALIDITY+HIGHESTMODSEQ, JMAP state, CalDAV sync-token, GCal syncToken).

All tables carry `account_id`; account rows carry `tenant_id` (always the local user on desktop). This costs nothing now and is required for multi-tenant cloud.

## 5. Mutations: ops log, proposals, approval

Every change — from the user, from sync, or from AI — is expressed as an **Op**:

```
SetFlags, AddToMailbox, RemoveFromMailbox, Move, Archive, Trash,
SaveDraft, UpdateDraft, DeleteDraft, Send,
CreateEvent, UpdateEvent, DeleteEvent, RespondToInvite
```

Lifecycle:

```
user action ─► Op(pending) ─► applied locally (optimistic) ─► executor ─► provider ─► Op(done | failed→retry/rollback)

AI action ──► Proposal{ops[], rationale, agent, source refs} (pending_review)
                 │
     user in Review queue: approve / edit / reject
                 │
              approved ─► ops enter the normal pipeline above
```

- Ops are durable (survive restart, work offline) and idempotent per provider.
- **Drafts are special**: an AI-proposed draft is *not* written to the server Drafts folder until approved — it lives only in the proposal. Option (setting): "sync AI drafts to server Drafts folder" so you can review from your phone; still never sent.
- **`Send` can never originate from an AI scope.** Only a human action in the UI produces a `Send` op. Approving an AI draft opens it in the composer pre-filled; the human hits send.
- Calendar proposals render as a diff ("Move *Standup* Tue 9:00 → 10:00; invites 4 attendees") and, optionally, as ghost events on the calendar view.
- Conflict handling: if the target changed since the proposal was made (etag/modseq mismatch), the proposal is marked **stale** and must be re-reviewed.
- Everything is written to an **audit log**: who/what proposed, what was approved, what was sent.

## 6. AI interface

Two complementary surfaces, both read-mostly.

### 6.1 Filesystem mirror (`~/Satchel/`)

A one-way export the AI can read with ordinary file tools — no special client needed:

```
~/Satchel/
  README.md                         # explains layout and how to propose changes
  mail/<account>/<mailbox>/<yyyy-mm>/<thread-slug>-<id>/
      thread.md                     # cleaned plaintext of the whole thread, quoted text collapsed
      meta.json                     # ids, participants, labels, flags, dates
      <n>.eml                       # hardlinks/copies of raw messages (opt-in)
      attachments/                  # opt-in, size-capped
  calendar/<account>/<calendar>/<yyyy>/<event-uid>.ics  + .md summary
  proposals/inbox/                  # AI may drop proposal JSON files here (alternative to MCP)
```

- Configurable scope (which accounts, how far back, include attachments or not).
- Written incrementally by the core after each sync; never read back as truth (except `proposals/inbox/`, which is validated and ingested as Proposals).

### 6.2 MCP server

Runs over stdio (for Claude Code / Desktop `claude mcp add satchel -- satchel mcp`) and localhost HTTP with a per-agent token.

Read tools: `search_messages`, `get_thread`, `list_mailboxes`, `list_events`, `get_event`, `find_free_time`, `get_attachment_text`.

Propose tools: `propose_reply`, `propose_draft`, `propose_labels`, `propose_archive`, `propose_move`, `propose_event`, `propose_event_change`, `propose_invite_response`. Each returns a proposal id and status; `get_proposal_status` lets the agent see what you approved or rejected.

Deliberately absent: `send`, `delete`, `empty_trash`, `change_settings`, anything that edits policies.

Per-agent policy (UI-managed, not agent-editable): which accounts it can see, date range, whether it can see attachments, rate limits on proposals.

### 6.3 Prompt-injection stance

Email content is attacker-controlled. The design assumes an agent may be manipulated by a message it reads, and limits blast radius structurally: no send, no delete, all writes are proposals, proposals show their source thread and full content before approval, and recipients outside the original thread are highlighted in review.

## 7. Provider adapters

```rust
#[async_trait]
trait MailProvider {
    fn capabilities(&self) -> MailCaps;                  // labels? threads? push? server search?
    async fn list_mailboxes(&self) -> Result<Vec<RemoteMailbox>>;
    async fn sync(&self, state: &SyncState, sink: &mut dyn SyncSink) -> Result<SyncState>; // delta
    async fn fetch_raw(&self, id: &RemoteId) -> Result<Bytes>;
    async fn apply(&self, op: &Op) -> Result<ApplyResult>;
    async fn watch(&self, tx: Sender<Change>) -> Result<()>; // IDLE / push / polling fallback
}
trait CalendarProvider { /* same shape for calendars/events */ }
```

| Provider | Mail | Delta mechanism | Push | Calendar |
|---|---|---|---|---|
| Gmail | Gmail API | `history.list` (historyId) | Pub/Sub (cloud) / polling (desktop) | Google Calendar API, `syncToken` |
| Microsoft 365 | Graph | `messages/delta` per folder | Graph subscriptions (cloud) / polling | Graph `calendarView/delta` |
| IMAP | IMAP4rev1/rev2 | CONDSTORE/QRESYNC, else UID scan | IDLE | CalDAV `sync-collection` (RFC 6578) |
| Fastmail | JMAP | `Email/changes` state | EventSource | JMAP Calendars if available, else CalDAV |

Sending: Gmail API / Graph `sendMail` / SMTP submission / JMAP `EmailSubmission`.

Initial sync strategy: headers + snippets for the last N months first (fast inbox), bodies in background, older history backfilled lazily.

## 8. Frontend (vanilla TS)

- Vite + TypeScript, no framework. Web Components (custom elements) for structure, a small hand-rolled store/event bus, CSS custom properties for theming.
- A single `Transport` interface: `invoke(command)` and `subscribe(events)`. Desktop implements it with Tauri IPC; cloud implements it with HTTPS + WebSocket. The UI never imports Tauri APIs directly outside that adapter.
- Virtualized lists for mailboxes with 100k+ messages.
- Message rendering in a sandboxed iframe (no scripts, CSP, remote images blocked by default, per-sender allow).
- Keyboard-first (Gmail/Superhuman-style shortcuts), command palette.

Views:
1. **Mail** — accounts/mailboxes sidebar, unified inbox, thread list, reading pane, composer (rich text + plain), search with operators (`from: to: has:attachment before:`).
2. **Calendar** — day/week/month/agenda, event editor, invite responses, free-time finder.
3. **Review** — the proposal queue: grouped by agent and thread, inline diffs, edit-before-approve, bulk approve for low-risk ops (labels/archive), reject with note (fed back to the agent via `get_proposal_status`).
4. **Settings** — accounts, AI agents & policies, mirror scope, privacy.

## 9. Cloud-readiness (what v1 must not break)

The hosted version = `apps/server` (axum) + same `core` + same `ui` served as a web app.

Rules enforced from day one:
- `core` depends on traits, not concretions: `Store`, `BlobStore`, `SecretStore`, `Clock`, `Scheduler`. Desktop: SQLite / files / OS keychain. Cloud: Postgres / S3 / KMS-encrypted secrets.
- The `api` crate is the only interface the UI and MCP use. If it's not in `api`, the UI can't do it — so the cloud web UI gets feature parity for free.
- Tenant/account ids everywhere (§4.2).
- Sync engine is runnable per-account as an independent task (cloud runs these as workers; push notifications from Gmail Pub/Sub / Graph subscriptions become cheap).
- MCP over HTTP with OAuth (remote agents connect to the hosted instance).
- Optional future: desktop app syncs against the hosted instance instead of providers directly.

## 10. Security & privacy

- OAuth tokens and passwords in the OS keychain (`keyring` crate); never in the DB or mirror.
- Optional at-rest encryption of the DB (SQLCipher) and mirror (off by default since the mirror's purpose is to be readable).
- HTML sanitized with `ammonia` and rendered sandboxed; tracking pixels blocked.
- MCP HTTP binds to 127.0.0.1 only, per-agent bearer tokens, revocable.
- Audit log of every approval and send.

### Provider approval risk (plan for it early)
- **Google**: Gmail scopes that read/modify mail are "restricted". Public distribution requires OAuth app verification, and — especially for the cloud version that stores mail on our servers — an annual third-party security assessment (CASA). Budget time and money; verify current requirements before launch. During development, test users on an unverified app work fine.
- **Microsoft**: multi-tenant Entra app registration + publisher verification; some orgs require admin consent.
- IMAP/CalDAV/JMAP have no such gate (app-specific passwords or provider OAuth).

## 11. Milestones

Order is chosen so every milestone is testable end-to-end against local test servers, before touching the OAuth-gated providers.

**Dev test servers** run natively on the dev machine (no Docker): Stalwart, a single binary that serves IMAP, SMTP, JMAP and CalDAV. `scripts/dev-servers` installs it if missing, starts it on fixed localhost ports with data in `.dev/` inside the repo, seeds test accounts and fixture mail/events, and supports `reset` to wipe back to a known state for each test run.

| # | Milestone | Done when |
|---|---|---|
| M0 | Skeleton | Workspace, Tauri shell, Transport, `api` schema, CI, SQLite migrations, `scripts/dev-servers` running local Stalwart |
| M1 | Read-only IMAP mail | Add IMAP account, initial + delta sync, IDLE, thread list + reading pane, sanitized rendering |
| M2 | Full mail client (IMAP/SMTP) | Compose/reply/forward, drafts, flags/move/archive via ops log, offline queue, full-text search |
| M3 | AI surface | Filesystem mirror, MCP server (read + propose), Review queue UI, audit log, agent policies |
| M4 | Gmail + Graph + JMAP mail | All three adapters, labels model, OAuth flows, keychain |
| M5 | Calendar | CalDAV, Google Calendar, Graph calendar, JMAP/CalDAV for Fastmail; calendar views, editor, invites; calendar proposals |
| M6 | Ship | Signing/notarization, auto-update, onboarding, performance pass (100k-message mailbox), Google/Microsoft verification submitted |
| M7 | Cloud | `apps/server`, Postgres/S3 stores, push-based sync workers, web UI, remote MCP with OAuth, billing |

## 12. Open questions

1. Trademark: formal clearance search, then file SATCHEL MAIL in class 9 (downloadable software) and class 42 (SaaS). Check domains and the `satchel-mail` names on GitHub and crates.io.
2. License — open-source core (trust story for an app that reads all your mail) with a paid hosted tier, or closed?
3. Mirror format details — is `thread.md` enough, or should we also offer a per-message JSON-lines export for bulk analysis?
4. Should approved low-risk proposals (labels, archive) be allowed to auto-apply under user-defined rules in v1.x?
5. Bundled local LLM (e.g. for summaries/triage) — later, or never?

## 13. Implementation status (M0–M6, 2026-10-23)

M0–M6 are built; M7 (hosted version) is on hold pending a design discussion.

| Milestone | Status | Verified against |
|---|---|---|
| M0 Skeleton | Done | Workspace, Tauri app, Transport (IPC + HTTP/WS), `api` crate, SQLite schema, `scripts/dev-servers` |
| M1 IMAP read | Done | Local Stalwart: CONDSTORE deltas, IDLE, threading, sanitized HTML |
| M2 Full mail client | Done | Local Stalwart: send (undo window), reply/forward, drafts sync, flags, move/archive/trash, full-text search |
| M3 AI surface | Done | MCP over stdio (24 tools), AI folder + file-drop proposals, Review queue, audit log, policies |
| M4 Gmail / Graph / JMAP | Done | JMAP live against Stalwart; Gmail and Graph unit-tested only (need OAuth client ids — `docs/OAUTH.md`) |
| M5 Calendar | Done | CalDAV live against Stalwart (sync-collection, PUT/DELETE, RSVP), emailed iMIP invites; Google/Graph unit-tested only |
| M6 Ship | Partly | macOS `.app` bundles with the CLI sidecar; perf pass at 100k messages (initial IMAP sync 83 s / 74 MB peak; list pages ≈2 ms, deep offsets ≤90 ms, counts ≈65 ms). Signing, notarization, update feed and provider verification need the owner's accounts/keys |

### Deviations from the plan above

Each decision, including these, has its own record in [docs/decisions/](decisions/README.md).

- **Test servers:** Stalwart alone (it also serves CalDAV and JMAP), so Radicale isn't used.
- **Store:** SQLite behind one module (`core/src/store.rs`) that contains all SQL; it is not yet a trait. `BlobStore` and `SecretStore` are traits. Extracting `Store` is the first M7 task.
- **IMAP identity:** `<uidvalidity>:<uid>:<mailbox>`. After a move we re-identify the copy by Message-ID among UIDs ≥ the target's previous UIDNEXT, so local ids survive moves we perform (async-imap doesn't expose OBJECTID).
- **Gmail:** labels map to mailboxes plus a virtual "All Mail"; deltas via `history.list`; older mail backfills a page per sync.
- **Graph calendar:** synced as concrete instances (`calendarView/delta`, −180/+540 days), not series masters; attendees respond via accept/decline endpoints.
- **Agents over HTTP:** `satchel serve` exposes `POST /api/agent/rpc` (bearer agent token) using the same command schema; MCP-over-HTTP is not implemented yet (stdio only).
- **Proposal de-duplication** and **email-invite RSVP** (`RespondToEmailInvite`) were added.
- **Large lists:** the thread list is a windowed cache addressed by offset, sized by `CountThreads`, holding at most 30 pages of 100; the scrollbar is compressed above 8M px so million-thread folders scroll. Refreshes re-read only the visible window and keep the top visible thread anchored. Agents page with keyset (`before` + `before_thread_id`).
- **Search:** SQLite FTS5 instead of a tantivy index, so text search, filters, counts and paging are one query with no result cap; headers are searchable before bodies download.
- **Updater:** behind the `updater` feature; `scripts/release.sh` enables it when an update URL and signing key are provided.
