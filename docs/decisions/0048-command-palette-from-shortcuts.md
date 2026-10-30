# 0048. The command palette is built from the shortcuts

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
The command palette (mod+k) had a hand-written list of a dozen entries:
places to go, compose, sync, dark mode. Almost all of them already had a
shortcut or sat one click away in the sidebar, it couldn't act on the open
conversation, and it matched only exact substrings. It added little, and
every new action would have needed adding in two places.

## Decision
The palette lists what the keyboard can do. `bind` (in `lib/ui.ts`) takes
an optional `when`, and `activeCommands()` turns the current view's
bindings, then the global ones, into commands, each with its key beside it.
Keys that only move around a list (j/k, arrows, Enter, Escape) are left
out, and so is anything whose `when` says there's nothing to act on.

Views add commands that aren't a single key with `commandSource`. Mail
adds "Move to <folder>" for each folder the conversation isn't already in,
"Move to new folder…" and "All mail with <person>", and names what they
act on ("Acting on “Lunch on Friday”"). The palette itself adds places
(every folder, each Settings page), New event, New contact, sync and dark
mode. As you type, it adds people from the contact search ("Mail with …",
"Write to …"), and its last entry is always a mail search.

Matching is fuzzy (`lib/fuzzy.ts`: in-order characters, ranked by
substring, word starts and runs), with matched letters in bold. The last
eight commands run are remembered in this browser's `localStorage` and
listed first when the box is empty.

## Consequences
A new shortcut appears in the palette with no extra work, so its `desc`
should read as a command. A shortcut that needs a target should pass
`when`, or the palette offers it where it does nothing. Recently used
commands are per device, which is fine for a convenience.
