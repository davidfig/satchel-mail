# 0035. Code layout: folders by domain, files under ~400 lines

- **Status:** Accepted
- **Date:** 2026-10-25

## Context

Each crate had grown as one flat folder, and a few files had become catch-alls:

- `store.rs` (2,900 lines) held every table.
- `commands.rs` (1,600) held one 470-line dispatch match plus every command.
- `api/lib.rs` (1,100) held every type.
- Provider files mixed products: Gmail with Google Calendar, Outlook mail
  with its calendar.
- In the UI, `mail.ts`, `conversation.ts`, `settings.ts`, `types.ts` and
  `styles.css` were each 450–680 lines.

Finding the code behind a bug meant searching inside large files rather than
going to a folder.

## Decision

- **Folders by domain, not by layer.** In `core`:
  - `mail/`, `calendar/`, `contacts/` and `agents/` hold the commands and
    logic for each domain.
  - `dispatch/` only routes a `Command` to its handler and checks access.
  - `engine/`, `ops/` and `ai_folder/` are the moving parts.
  - The store stays one layer (`store/`, one file per table group) because
    all SQL stays together for the Postgres port (0004, 0018).
- **Providers by vendor.** `imap/`, `jmap/`, `google/`, `microsoft/` and
  `dav/` each contain mail, calendar and contacts as separate files where the
  vendor offers them.
- **Mirrored types.** `api` has one file per domain, mirrored one-to-one by
  `ui/src/types/`.
- **UI.** `app/` (shell, palette), `views/mail/` and `views/settings/` as
  folders, and `styles/` split along the old section comments, imported in
  the same order.
- **A soft limit of about 400 lines per file.** Split when a file holds more
  than one concern, not to hit a number. Unit tests may live beside the
  code. Integration tests are split by area, with shared fakes in
  `tests/support/`.
- **Stable public paths.** Folder `mod.rs` files re-export what other crates
  use, so `satchel_core::store::Store` and `use satchel_api::*` are
  unchanged. Each folder's module doc says what's in it.

## Consequences

- More files (about 40 became about 120). Navigation is by folder, and
  `CLAUDE.md`'s "Where things are" maps them.
- A split type (`impl Engine`, `impl Store`) spreads over several files.
  Helpers shared between those files are `pub(crate)` or `pub(super)`
  rather than private.
- `dispatch/mod.rs` stays one exhaustive match, so the compiler still checks
  that every command is handled. Arm bodies that did real work moved into
  their domain's files.
- Older decision records mention the old file names; they describe the code
  as it was.
