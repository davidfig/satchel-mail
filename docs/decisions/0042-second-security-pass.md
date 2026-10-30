# 0042. Hardening after the second security pass

- **Status:** Accepted
- **Date:** 2026-10-30
- **Extends:** 0040. **Amends:** 0022, 0029, 0034

## Context

A second review on 2026-10-30, after 0040, found the following problems:

- **Sync stoppers:** two emails that stop sync for good:
  - one with a References header so long it ran past SQLite's limit on query parameters;
  - an invite whose duration overflows and crashes the calendar code.
- **Slow HTML:** HTML handling that took quadratic time.
- **Trust signals:** the "not forged" check (0034) could be fooled. Forged "sent" mail created contacts. Matching Message-IDs merged threads.
- **Agent boundary:**
  - the folder export ignored the date limit for individual messages;
  - agent tokens in folder files could be read and reused by any other folder tool;
  - `labels` proposals could move mail to Trash without a warning.
- **Network:** JMAP sent credentials to any https host, despite what 0040 said.
- **Leftovers:** deleted mail left its raw message on disk.

## Decision

**Hostile input can't stop sync:**

- **References:** only the last 100 are kept, and thread lookup uses `json_each`.
- **Bad messages:** a message that fails to store for a data reason is logged and skipped. Database failures still stop the pass.
- **Calendar times:** durations use checked arithmetic and are limited to about 10 years.
- **HTML:** HTML handling is linear. HTML longer than 2 MiB is cut, with a note in the text.

**Trust needs proof:**

- **Images and Junk:** the per-sender image allowance and sender rules that pull mail out of Junk need a DMARC pass, or a DKIM pass aligned with the From domain. Authentication-Results is parsed by resinfo, with comments stripped. Missing results count as unverified. The weaker "not forged" test (no failures) stays for warnings and invites.
- **Contacts:** "sent by the user" means the user's address in From *and* in a Sent-role mailbox. Only those messages make automatic contacts or "known" correspondents.
- **Threads:** an incoming message joins its most recent matching thread and doesn't merge other threads. Two rows count as copies only when the Message-ID, sender and snippet all match.
- **Invites:**
  - **Acting on a stored event:** an invite needs sender = ORGANIZER and a message that isn't forged, and the card shows the stored event's title.
  - **Removing an event when mail is marked spam:** the ics organizer, the stored organizer and the sender must all agree.
  - **Copies:** invite copies are filtered on unfolded lines, so folding can't smuggle in alarms or extra components.

**Agents:**

- **No tokens in folder files.**
  - Files carrying `agent_token:` or any `stk_…` are refused, with an error written to `processed/`.
  - Every folder file comes from the folder agent. Connected agents use MCP or `/api/agent/rpc`.
  - This amends 0022 and 0029.
- **`processed/` is not authoritative.** Every agent can edit it, so the user's review note is no longer written there.
- **Date limit:** the folder export applies it per message, including to `.eml` files, attachments and `meta.json`.
- **Labels proposals:** they warn when they touch Trash, Junk, Sent, Drafts or Archive, or when a folder account will apply them as a move. The UI no longer counts `labels` as low-risk.
- **Attachments:** agents without `allow_attachments` get no attachment content, text included.
- **Agent drafts:** they lose `body_html`, and attachment types must be `type/subtype`.
- **Limits:** `find_free_time` covers at most 400 days and 500 slots.
- **Hiding what's out of scope:** names outside the agent's scope get the same error as missing ones, and the duplicate-contact warning only looks within scope.
- **User-only commands:** settings and agent-management commands need the app's own process, not a `satchel call` running beside it.
- **Attribution:** approvals record the agent id.

**Network** (this corrects 0040, which overstated the JMAP rule):

- **JMAP:**
  - the API, upload and event-source URLs must pass the same check as DAV;
  - downloads may also use a short list of known hosts (`fastmailusercontent.com`);
  - internal addresses are refused unless the session itself is internal.
- **Same-site check:** hosts under shared-hosting suffixes (`duckdns.org`, `github.io`, …) count only as their own origin.
- **Graph:** Graph links and cursors must be on `https://graph.microsoft.com/`.
- **Plaintext:** IMAP or SMTP without TLS may only log in to a loopback host.
- **Unsubscribe client:** it ignores proxy settings, and NAT64 and 6to4 addresses count by the IPv4 inside them.
- **Timeouts and caps:**
  - the OAuth listener reads each connection with a timeout;
  - the IMAP steps before IDLE have timeouts;
  - JMAP paging is capped per pass.

**Storage:**

- Deleting messages or accounts queues their blobs, and a worker deletes the ones nothing uses any more. Orphans are swept at startup.
- A failed secret delete is reported.
- The `sent` audit entry is written when the op completes and lists recipients.
- Retry only resets failed or pending ops.

**Web mode:**

- **Session link:** the printed link uses `#session=`, and `?session=` still works. A new token is saved only after a test call succeeds.
- **Framing:** the app can't be framed (`frame-ancestors 'none'`, `X-Frame-Options`), and framed API calls are refused.
- **Links:** middle-click goes through the same link check as a click.
- **Unsubscribe:** the dialog shows the full message it will send.

**Windows** (this corrects 0040): the non-unix folder code still checks paths before using them, and can't see link counts. It therefore doesn't export `.eml` files or attachments.

## Consequences

- **Small senders:** senders without DMARC or aligned DKIM lose the per-sender image allowance and Junk rescue.
- **Contacts:** accounts without a Sent-role folder no longer produce automatic contacts from their own mail.
- **Connected agents:** agents that used the folder with a token must switch to MCP or the HTTP endpoint.
- **Rejection notes:** they appear only in Satchel, no longer in `processed/`.
- **CLI:** `satchel agents create`, `satchel agents revoke` and similar commands fail while the app is running.
- **JMAP:** servers that put their API on another site, and remote plaintext logins, now fail with an error.
- **Skipped messages:** a message that can't be stored is skipped, so it won't appear in Satchel.
- **No trust until learned:** IMAP and JMAP accounts on servers that aren't built in trust no sender until about 20 recent messages show the same authserv-id. A server that adds no Authentication-Results header never earns trust.
- **iCloud:** iCloud puts each check in its own header. Only the topmost header is read, so some iCloud mail stays unverified.
- **Windows:** the AI folder refuses every reparse point, so it can't sit in a OneDrive Files On-Demand folder. The Windows code type-checks against `windows-sys` but hasn't been run on Windows.
- **Agent thread lists:** a filtered page can hold fewer threads than its limit.

## Follow-up the same day

The items 0042 first left open were fixed:

- **Web-mode links:** links in mail bodies are rewritten to the app's own `#follow=` page, which opens in a new tab. That page shows the address and always asks before navigating, so the link check works even when the browser runs no handlers in the sandboxed frame (Safari). Desktop is unchanged.
- **Authserv-id:** only the topmost Authentication-Results header is read. For trust, its authserv-id must belong to the account's receiving server:
  - **Built in:** Gmail `mx.google.com`; Microsoft `*.protection.outlook.com`, `outlook.com` and its id-less form; Fastmail `*.messagingengine.com`; iCloud `*.icloud.com` and `*.me.com`.
  - **Learned:** for other accounts, an id seen on at least 20 messages and on more than half of the last 200 that weren't sent by the user.
  - **Storage:** this is stored in `messages.auth_serv` (schema 4).
- **Teredo:** Teredo addresses are judged by the client IPv4 they carry.
- **Agents see only what's inside their date limit:**
  - the folder's INDEX.md, mailbox listings and CONTACTS.md, and agents' `list_threads` and search results, summarise only messages inside the limit (`mail/visible.rs`);
  - threads with nothing inside the limit are left out.
- **Not-found errors:** scope refusals for agents return the same "not found" error as a missing item. Policy refusals that reveal nothing ("calendar not enabled", "attachments not shared") stay "forbidden".
- **Windows folder access:**
  - every directory below the root is opened without following links and held without delete-sharing, so it can't be swapped while in use;
  - every handle's final path is checked against the expected path;
  - files with more than one link are refused;
  - writes are renamed by handle;
  - junctions are removed themselves, never followed;
  - `.eml` files and attachments are exported on Windows again;
  - `windows-sys` is a direct dependency on Windows (0003).
