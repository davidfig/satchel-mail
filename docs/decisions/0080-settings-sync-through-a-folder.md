# 0080. Settings sync between computers through a shared folder

- **Status:** Accepted
- **Date:** 2026-11-11

## Context

Everything Satchel knows lives in one computer's database. On a second
computer the person sets up their accounts, agent permissions, smart folders,
spam rules and preferences again, and an agent revoked on one computer still
works on the other. The hosted version (0018), which would fix this, is on
hold, and it would mean holding mail in the cloud.

Most of this is small and changes rarely. It can travel without a server:
many people already have a folder that follows them between computers
(iCloud Drive, Dropbox, OneDrive, a golemdocs vault).

## Decision

**A sync folder, chosen per computer.** Settings → Sync (or `satchel settings-sync`)
points Satchel at a folder that something else keeps in step between
computers. Satchel doesn't know or care what that is. Mail, calendars,
contacts, sync state and credentials never go there; each computer still
talks to the providers itself.

**One file per computer, nobody writes anyone else's.** Each computer writes
`<device id>.json` and only reads the others. Folder sync tools resolve two
computers writing the same file by keeping one copy (golemdocs: the file
wins) or by making "conflicted copy" files; with one writer per file neither
can lose a change. Writes are atomic (a dot-named temporary file, then a
rename), and dot-names are ignored, which golemdocs and iCloud also do.

**Each file is a snapshot, not a log.** A file holds every synced item as
that computer last saw it: key → value, the time it changed and the device
that changed it. A computer merges by taking, per key, the newest change
across all the files and itself (ties go to the larger device id). Removals
are kept as tombstones. Snapshots don't grow with edits, need no compaction,
and reading one twice is harmless.

**Local changes are found by comparison, not hooks.** Each pass builds the
current items from the database and compares each with the local form
recorded when it was last synced (`sync.state` in `settings`). A difference is
a change made here, stamped now. So no command has to remember to report to
sync, and an item that one computer can only partly express (an agent limited
to an account this computer doesn't have) isn't mistaken for a local change.

**What syncs (phase 1):**

| Item | Key | Applied as |
|---|---|---|
| Preferences (theme, notifications, remote images, alerts, places, poll interval, AI drafts on the server) | `settings` | merged into Settings; disk and AI-folder settings stay per computer |
| Accounts | `account:<email>` | offered, not added: "On your other computers" with Sign in, which fills in the server setup. Passwords and OAuth tokens never sync |
| Agents | `agent:<id>`; the built-in AI folder agent doesn't sync, since its access follows each computer's AI folder settings | name and policy; accounts by email. An agent from another computer arrives with no token (AI agents → New token makes one here). Revoking anywhere revokes everywhere |
| Smart folders | `smart_folder:<id>` | same id on every computer |
| Spam rules | `sender_rule:<email>` | `SetSenderRule`, so mail is moved as usual |
| Favorite senders | `favorite:<email>` | `SetFavoriteSenders` |
| AI proposals | `proposal:<id>` | added for review, or the outcome taken; see below |

**Proposals (phase 2)** sync as `proposal:<id>`, with the same id everywhere,
so an agent's proposal can be reviewed on any computer:

- Local ids are written as ids every computer shares: the account's email
  and the provider's own id (messages also carry their Message-ID header, so
  a message the provider renumbered is still found), and read back as this
  computer's ids. Source threads travel as their newest message.
- A proposal whose messages or events haven't synced to a computer yet waits
  in the sync state and is tried again every pass.
- **Only the computer where it's approved carries it out.** The others take
  the outcome (status, note, time, the edited action) and never run it. A
  decision beats an older "pending" from elsewhere: the computer that decided
  sends its decision out again.
- Staleness on another computer is measured from when the proposal arrived
  there.
- Contact proposals stay on the computer that made them: contacts made from
  mail have no id another computer shares. So does a draft with more than
  256 KB of attachments.
- Proposals sync for 60 days after they're made; then the sync forgets them
  (each computer keeps its own copy). Proposals are never removed by sync.

**Encrypted end to end.** The folder sync service sees only ciphertext.
Turning sync on makes a random 256-bit key and shows it once as a sync code
(base32, grouped). Another computer joins by choosing the same folder and
typing the code. The key lives in each computer's secret store (0052), never
in the folder. Each file is sealed with ChaCha20-Poly1305 and a fresh random
96-bit nonce (via `aws-lc-rs`, already in the build for TLS), with the device id as associated
data so a file can't be passed off as another computer's. The folder's
`satchel-sync.json` holds a key check (a BLAKE3 keyed hash of a fixed label),
so a wrong code is refused at join, not discovered later as garbage. The
files are base64 text, so tools that only sync documents (golemdocs) carry
them.

## Consequences

- Sync latency is the folder tool's: seconds for iCloud, about a minute for a
  golemdocs vault (it checks the account once a minute).
- Last change wins per item, by each computer's clock. Two computers editing
  the same smart folder within a clock skew of each other can pick the
  "wrong" one; acceptable for settings.
- Losing every computer's copy of the sync code means turning sync off and on
  again (a new key); the old files are then unreadable and are replaced.
- Removing an account on one computer withdraws the offer elsewhere but never
  removes the account there.
- An agent's access to an account this computer doesn't have is kept: editing
  the agent here carries that account over from the synced value, and adding
  the account here gives the agent access to it.
- Smart folders sync their place as an index in the sidebar, so the order
  follows; folders made on two computers at once may swap until one moves.
- With iCloud Drive's "Optimize Mac Storage", another computer's file can be
  moved off the disk; Satchel asks iCloud for it back (`brctl download`) and
  reads it on a later pass.
- A computer is listed by its name in System Settings on a Mac, or its host
  name elsewhere, as it was when it joined.
- Approving the same proposal on two computers before either has synced the
  other's decision carries it out twice. Moves, labels and flags come out the
  same; a new event or a draft is made twice. Each computer's Review list
  drops it within a pass or two of a decision anywhere.
- The hosted version could later read and write the same snapshots.
