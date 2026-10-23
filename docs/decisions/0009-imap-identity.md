# 0009. IMAP message identity and move re-identification

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

IMAP UIDs are per mailbox, and async-imap doesn't expose RFC 8474 OBJECTID, so
a message has no server-wide id. Moves would otherwise give it a new local id,
breaking open views and proposals.

## Decision

Remote id is `<uidvalidity>:<uid>:<mailbox>`. Before a MOVE we note the target's
UIDNEXT; afterwards we read Message-IDs of UIDs ≥ that value and re-key the
local message to its new copy (Stalwart's HEADER search proved unreliable for
this). Optimistic placements the server never confirms are dropped on the next
sync. Flag sync uses CONDSTORE when available; deletions are found with
`UID SEARCH ALL`.

## Consequences

- Moves made by other clients appear as delete + insert (new local id).
- Large fetches are streamed, not collected (a 100k-message sync peaks at ~74 MB).
