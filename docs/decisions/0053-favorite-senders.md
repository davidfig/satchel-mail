# 0053. Favorite senders: a local list of addresses, highlighted everywhere

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

Flags (stars) mark single messages and sync to the server. There was no way
to say "mail from this person matters": to see everything from someone you
had to open their person view, and nothing in the inbox set their mail apart
from the rest.

## Decision

**A list of addresses.** Favorites live in `favorite_senders`, one row per
lowercased address, global across accounts, like `sender_rules` (0024). We
key them by address rather than by contact so any sender can be a favorite,
including one who isn't a contact yet. Favoriting from the person view adds
all of that person's addresses; from a conversation it adds the sender's.

**Local only.** No provider has a matching concept (Gmail's "important" is
per message and the server decides it), so favorites aren't synced or written
to an address book. They don't change where mail goes.

**What it does.** A conversation is a favorite when any of its messages is
*from* a favorite address (mail you sent to them doesn't count). Such rows
get a tint and a heart in the list (`ThreadSummary.favorite`).
`is:favorite` finds them, and the sidebar shows a Favorites entry
(`g f`) once there's at least one. You add or remove a favorite from the
row menu, the reader's More menu, the command palette, the person view and
Settings → Favorites.

**Agents.** `ListFavoriteSenders` and `SetFavoriteSenders` are for the user
only. Agents see the result, not the list: `favorite` on thread summaries,
in the AI folder's `index.jsonl`, and through `is:favorite` searches. No
proposal kind can change favorites. They are the user's settings, and agents
never change settings.

## Consequences

- Highlighting reads one extra indexed lookup per listed thread.
- Settings → Favorites lists them all, where you can remove one or add an
  address you haven't had mail from yet.
- Notifications ignore favorites for now. "Notify only for favorites" is an
  obvious next step, built on the same table.
