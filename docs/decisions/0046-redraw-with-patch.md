# 0046. Views that redraw on their own use `patch`, not a framework

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

Views were drawn with `replace`: throw the old nodes away, build new ones.
That's fine for a page drawn once, but many views redraw while you look at
them — on every sync tick, every agent write, every keystroke. Each redraw
dropped the hover under the pointer, closed tooltips and restarted
animations, so the sidebar, Settings → Accounts and others blinked during a
sync. We considered moving the UI to React for its diffing, but that means
rewriting every view and giving up 0002 for a problem a small helper solves.

## Decision

`patch(el, …children)` in `ui/src/lib/dom.ts` is our diffing: build the new
tree with `h()` as usual, and `patch` walks the live DOM, keeping nodes that
match and copying over attributes, text, `on`/`listen` handlers and form
state (value, checked, selected; a focused field keeps what's typed). A node
you keep yourself, passed straight in and already there (a search box),
stays as it is.

Anything that redraws on its own uses `patch`: the sidebar, the thread list
and mail header, Settings → Accounts / AI agents / Activity, AI Proposals,
Contacts (list and contact), the calendar's side panel and grid, the command
palette, address suggestions, invitation cards.

For `patch` to be safe, render code must not depend on the identity of the
nodes it builds, since they may be thrown away: listeners go through `on` or
`listen` (never `addEventListener`), and handlers read `e.currentTarget`
rather than a captured element. Pieces that need stable elements are built
once and updated directly (the calendar header, whose "…" menu holds on to
its buttons).

`replace` stays for things drawn once or only by the user's own action,
and for forms that keep references to their fields (Settings → General,
AI folder, Privacy; the contact editor). Those don't redraw while edited.
The reader also replaces whole conversations: their HTML lives in iframes,
which reload if touched, and it already stages the new page behind the old.

## Consequences

- No framework, no rewrite; views keep their render-from-scratch style.
- Writing a view now has two rules (`on`, not `addEventListener`;
  `currentTarget`, not captured elements). Breaking them fails quietly: a
  handler acting on a detached node. Check new views that redraw on events.
- If views grow much more stateful, revisit this (0002) rather than piling
  more onto `patch`.
