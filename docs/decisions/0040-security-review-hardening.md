# 0040. Hardening after the October security review

- **Status:** Accepted; extended and corrected by 0042
- **Date:** 2026-10-30

## Context

A security review on 2026-10-30 found gaps in four places:

- **AI folder:** 0032's symlink checks walked paths, then reopened them by name. A symlinked `proposals/inbox` made Satchel delete files in the target directory. Hard links and a swapped directory got past the checks.
- **Proposals:** `forward_of`, mailbox ids and source threads skipped the agent's scope checks. An `UpdateEvent` could add attendees with no warning.
- **Mail content:** a `List-Unsubscribe` mailto could make the user's account send attacker-written mail to anyone, including extra recipients. Invites were matched by UID alone, and saved attachments weren't marked as downloaded.
- **Local server and network:** the `serve` cookie worked from curl once copied. DAV sent the password wherever the server pointed it. The data directory and secrets file followed the umask. Responses had no size limit.

## Decision

**AI folder** (extends 0032):

- **Walking paths:** every access opens the root, then each directory with `openat(O_DIRECTORY|O_NOFOLLOW)`, and acts relative to that handle (`openat`, `renameat`, `unlinkat`). A symlink anywhere below the root is refused, and nothing is deleted or written through one.
- **What gets read:** files with more than one hard link are refused. Agent files are capped at 1 MiB.
- **Symlinked folders:** `ensure_layout` replaces a symlinked `proposals/`, `inbox/` or `processed/` with a real directory.
- **Tokens:** `stk_…` tokens are scrubbed from everything written back to `processed/`.
- **Status updates:** the processed file written for each proposal is recorded on the proposal (`processed_file`, schema 3). Status updates go only to that file, so a copy an agent plants never receives the user's review note, even after a restart. Proposals from before the migration get no further status updates in the folder.
- **Calendar and contacts:** they can be turned off for the AI folder (`include_calendar` and `include_contacts`, on by default). When off, the section is removed from the folder, person-file edits aren't read, and the folder agent's `allow_calendar` and `allow_contacts` follow the setting.
- **Export names:** names are capped at 120 bytes plus a hash, and stay distinct when compared case-insensitively. They can't take reserved names such as `INDEX.md`.
- **Export failures:** one bad item is logged and skipped, so it doesn't stop the whole pass.
- **Exported text:** mail bodies are fenced in `thread.md`, and single-line fields have newlines collapsed, so mail can't forge structure.

**Agents:**

- **Central check:** `Engine::handle` checks a second, separately written allow-list (`dispatch/access.rs`). A write command can't reach an agent even if `allowed_for_agent` is widened by mistake.
- **`forward_of`:** goes through `check_message`. It must be in the draft's account and respects `allow_attachments`. Sending refuses a forward from another account, for every caller.
- **Events:** `UpdateEvent` warns about added or removed attendees and organizer changes, and refuses a UID change. Agent `CreateEvent` can't set its own UID or organizer.
- **Scope:** mailboxes must belong to the messages' account and to an account the agent may see. Source threads respect the date cutoff. `Info` hides `data_dir` from agents.

**Mail content:**

- **Unsubscribe by mailto:** only a single plain address is accepted. The UI shows the exact To, subject and body before sending, and warns when the domain differs from the sender's.
- **Invites:** an invite matches a stored event only if the UID and organizer both match. An email reply goes out only when the ORGANIZER is the sender. A copy stores only the invited series, without alarms.
- **Saved attachments:** files saved from the desktop app get `com.apple.quarantine` on macOS, or `Zone.Identifier` on Windows.
- **Risky attachments:** the list was widened (scripts, shortcuts, disk images, archives). Trailing dots no longer hide an extension, and bidi or control characters are stripped from names.
- **Remote images:** a per-sender image allowance only applies to mail that isn't forged (the same test sender rules use, 0034).

**Local server:**

- **There is no session cookie any more.** Cookies ignore ports, so the cookie reached every server on 127.0.0.1, and a copied cookie worked from curl with a forged `Origin`. Opening the printed `?session=` link now saves the token in that origin's `localStorage`, which is kept per port, and removes it from the address bar. Every user call sends the token as `Authorization: Bearer`. The WebSocket sends it as the subprotocol `bearer.<token>`, and the server answers `satchel`. Browser calls must still come from our own origin.
- **Token rotation:** the token is replaced after 30 days or with `satchel serve --new-session`. A `SATCHEL_SESSION_TOKEN` must be at least 32 characters.
- **Dev mode:** the Vite dev proxy binds to 127.0.0.1. It adds the bearer only for requests from the dev origin.
- **Files:** the data directory is 0700. The secrets file is created 0600 before anything is written to it. A damaged secrets file is an error, never read as empty.

**IMAP limits:**

- **Large messages:** during sync, messages whose `RFC822.SIZE` is over 64 MiB come down as headers only. Their content is fetched when opened, up to 160 MiB; larger ones point the user to webmail. async-imap holds a whole literal in memory (its own cap is 512 MiB), so this bounds memory with honest servers. A server that lies about sizes is still held to async-imap's cap.
- **Slow servers:** apart from the silence timeout, each response (everything read between two of our writes) must average 1 KiB per 10 minutes. A server trickling a byte every few minutes is cut off. Progress lines on long commands, such as Dovecot's search progress, send far more. IDLE connections have no limit, as before.

**Network:**

- **Credentials:** DAV and JMAP send them only to the account's origin or to https hosts on the same site, and never downgrade to http. Same-site rather than same-origin is needed for iCloud's `pNN-caldav` hosts and Fastmail's download host.
- **Size limits:** HTTP bodies are capped at 16 MiB for tokens and sessions, 64 MiB for API and DAV responses, and 160 MiB for raw messages.
- **Unsubscribe links:** one-click unsubscribe only reaches public https hosts. That's checked at DNS resolution and on every redirect.
- **OAuth:** a callback with the wrong `state` is ignored, not fatal.
- **Send retries:** Gmail and Graph sends aren't retried on 503. That's reported as maybe-sent (0030).
- **URLs:** provider ids are percent-encoded in URL paths.

## Consequences

- **Web mode:** browsers that were signed in with the old cookie need to open the printed link once more. After that, the same link is needed every 30 days or after `--new-session`. Scripts use the bearer token.
- **Dev mode:** it runs on `http://127.0.0.1:7780`, not `localhost`.
- **Invites:** when the sender isn't the organizer (a delegate sent it), the reply can't be sent by email. Respond through a synced calendar instead.
- **DAV:** a DAV server that redirects to another site fails with an error instead of getting the password.
- **IMAP:** messages over 64 MiB show their headers until opened. Those over 160 MiB can't be opened in Satchel.
- **`libc` is now a direct dependency** of core (unix) and the desktop app (macOS), for `openat` and `setxattr`.
