# Decisions

One file per decision. Copy `template.md`, use the next number, and add it here. When a decision is replaced, mark the old record "Superseded by NNNN".

| # | Decision | Status |
|---|---|---|
| 0001 | [Local-first desktop app around a shared core](0001-local-first-shared-core.md) | Accepted |
| 0002 | [Rust core and a vanilla TypeScript UI](0002-rust-core-vanilla-ts-ui.md) | Accepted |
| 0003 | [Build from scratch; depend only where it pays](0003-dependencies.md) | Accepted |
| 0004 | [SQLite as the local store, all SQL in one module](0004-sqlite-store.md) | Accepted |
| 0005 | [Content-addressed raw message storage](0005-content-addressed-blobs.md) | Accepted |
| 0006 | [AI agents can only read and propose](0006-agents-propose-only.md) | Accepted, amended by 0023 |
| 0007 | [MCP over stdio as the agent interface, plus an AI folder](0007-mcp-and-ai-folder.md) | Superseded by 0022 |
| 0008 | [Optimistic local changes with a durable op queue](0008-ops-queue.md) | Accepted |
| 0009 | [IMAP message identity and move re-identification](0009-imap-identity.md) | Accepted |
| 0010 | [Gmail labels as mailboxes, with a virtual All Mail](0010-gmail-labels.md) | Accepted |
| 0011 | [Microsoft Graph calendars synced as instances](0011-graph-calendar-instances.md) | Accepted |
| 0012 | [Native local test servers, no Docker](0012-native-test-servers.md) | Accepted |
| 0013 | [Windowed thread list and stable paging](0013-windowed-thread-list.md) | Accepted |
| 0014 | [Full-text search with SQLite FTS5](0014-sqlite-fts5-search.md) | Accepted (supersedes the tantivy index) |
| 0015 | [Credentials in the OS keychain](0015-secrets.md) | Accepted |
| 0016 | [Name: Satchel Mail](0016-name.md) | Accepted |
| 0017 | [OAuth with PKCE and a loopback redirect](0017-oauth.md) | Accepted |
| 0018 | [Hosted (cloud) version](0018-hosted-version.md) | Proposed — on hold |
| 0019 | [Conversations as a chat-style stream](0019-conversation-stream.md) | Accepted |
| 0020 | [Logo: a leather satchel with an envelope flap](0020-logo.md) | Accepted |
| 0021 | [Shared UI components and our own tooltips](0021-shared-ui-components.md) | Accepted |
| 0022 | [Files are the main way agents work with Satchel](0022-files-first-agents.md) | Accepted |
| 0023 | [Users choose how much agents may do on their own](0023-agent-autonomy-levels.md) | Accepted |
| 0024 | [Spam: sender rules outrank the server, and the core warns about phishing](0024-spam-handling.md) | Accepted |
| 0025 | [New-mail notifications in the system notification center](0025-new-mail-notifications.md) | Accepted |
| 0026 | [Contacts: generated from mail, edited by you, synced with your accounts](0026-contacts.md) | Accepted |
| 0027 | [A person view built on `with:` search](0027-person-view.md) | Accepted |
| 0028 | [Folders: managed in Satchel, pushed through the op queue](0028-folders.md) | Accepted |
| 0029 | [Agents edit contacts by editing their files](0029-editable-contact-files.md) | Accepted |
| 0030 | [Sends go out at most once](0030-sends-go-out-at-most-once.md) | Accepted |
| 0031 | [Versioned schema with ordered migrations](0031-schema-migrations.md) | Accepted |
| 0032 | [Local attackers: the web session and AI-folder symlinks](0032-local-attack-surface.md) | Accepted |
| 0033 | [Writes to the server change only what the user changed](0033-writes-change-only-what-changed.md) | Accepted |
| 0034 | [Sender rules act on the server only for mail they're sure about](0034-sender-rules-act-only-on-authentic-mail.md) | Accepted |
| 0035 | [Code layout: folders by domain, files under ~400 lines](0035-code-layout.md) | Accepted |
| 0036 | [Download all mail without attachments, within a storage limit](0036-message-content-and-storage-limit.md) | Accepted |
| 0037 | [The user's requests go before background sync](0037-user-requests-before-sync.md) | Accepted |
| 0038 | [Background reads use their own connection](0038-separate-read-connections.md) | Accepted |
| 0039 | [Newer mail first; older mail syncs over time](0039-newer-mail-first.md) | Accepted |
| 0040 | [Hardening after the October security review](0040-security-review-hardening.md) | Accepted |
| 0041 | [Changing the AI folder's location moves it](0041-moving-the-ai-folder.md) | Accepted |
| 0042 | [Hardening after the second security pass](0042-second-security-pass.md) | Accepted |
| 0043 | [Convert HTML mail to text ourselves](0043-html-to-text-in-tree.md) | Accepted |
| 0044 | [Approved drafts can be sent straight from Review](0044-send-from-review.md) | Accepted |
| 0045 | [Agents can propose merging contacts](0045-agents-propose-contact-merges.md) | Accepted |
