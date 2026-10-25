# 0029. Agents edit contacts by editing their files

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

The AI folder writes one file per person in `contacts/people/` (0026). Files
are the main way agents work with Satchel (0022), and the natural thing for
an agent to do with a contact file is edit it, or add a new one. Until now
that did nothing: the folder was a one-way copy, and the next pass wrote
over the edit or deleted the new file. Agents had to learn a separate
`save_contact` proposal format instead.

Agents still may not change anything directly. An edit has to go through
the same proposal path as everything else, so the user's agent policy
decides whether it applies on arrival or waits in Review (0023).

## Decision

- **A new file** in `contacts/people/` (a header with `name:` or `emails:`
  and no `contact_id`) becomes a `save_contact` proposal for a new contact.
- **An edited file** becomes a `save_contact` proposal for that contact.
  Only the header fields the agent changed are proposed. The base is the
  file as the mirror last wrote it, ours is the contact now, theirs is the
  edited file, merged with the same three-way merge address-book sync uses.
  So a change the user or a server made after the agent read the file is
  kept, not reverted. The header has no labels, so labels come from the
  contact.
- **The file then moves** to `proposals/processed/` with `status:` added,
  exactly like an inbox file, and the person's file is written again from
  the store. It shows the change once the proposal applies.
- **Deleting a file does nothing**; the next pass writes it back. Agents
  can't delete or merge contacts.
- `why:`, `agent:` and `agent_token:` work in the header as in inbox files.
  Without a token the proposal comes from the built-in AI folder agent and
  is subject to its settings.

**Telling edits from stale copies.** The engine remembers what it last wrote
to each person file (in memory). A file that differs from that is an edit.
After a restart there's no memory, so a file counts as edited only if its
`version:` is the contact's current version and its fields differ; an older
file is just out of date and is overwritten. A file with an edit waiting to
be read is never overwritten or deleted by the mirror pass.

Contacts are rewritten whenever they change (`ContactsChanged`), not only
on the 20-second index pass, so the folder catches up within seconds.

## Consequences

- Agents can maintain contacts with ordinary file edits, and the
  `save_contact` inbox proposal still works.
- An edit the agent makes while Satchel isn't running is picked up on the
  next start, as long as nothing changed that contact in between.
- The folder is no longer purely one-way. This adds a second place the core
  reads agent input from, with the same validation and attribution as the
  inbox, so it widens nothing an agent could already do.
- Renames and new addresses still always wait for review (0026), so an
  injected agent can't quietly redirect mail by editing a file.
