# 0027. A person view built on `with:` search

- **Status:** Accepted
- **Date:** 2026-10-24

## Context
There was no way to see everything exchanged with one person. `from:` only
finds mail they sent, `to:` only mail addressed to them, and both match part
of a name or address, so `from:bob` also finds `jimbob@…` and look-alike
scam addresses.

## Decision
- A `with:` search operator matches messages the person sent or received
  (From, To, Cc, Bcc). `from:`, `to:` and `with:` take a comma-separated list
  of addresses, any of which may match. A full address (`bob@x.test`) matches
  exactly; anything else (`bob`, `@x.test`) still matches part of a name or
  address.
- Clicking a name or avatar in a conversation opens a person view in the mail
  list: their addresses, counts of conversations and of mail each way, the
  meetings they organize or attend, and their conversations filtered by
  All / From them / Attachments. The open conversation stays open, and a back
  button returns to the previous list.
- The list is the ordinary thread list running a `with:` search, so selection,
  keyboard shortcuts and bulk actions work unchanged. The header comes from a
  new user-only `get_person` command. Agents get the same conversations through
  `with:` in `search_threads`, and need no new tool.
- The view covers every address of a contact (0026): clicking any one of
  them, or "All mail" on the contact, lists mail to and from all of them, and
  the person view links back to the contact. A contact's message count is the
  person view's received + sent (one shared query, drafts excluded), so the
  two always agree.

## Consequences
- Exact address matching makes look-alike addresses show up as separate
  people, which is what you want when checking a suspicious sender.
- Exact recipient matches look for the address in the stored address JSON
  with LIKE. That scans messages, which is fine at today's sizes. If it gets
  slow, add a table of addresses per message.
- Meetings are found by searching each event's ICS text for `mailto:<address>`.
  Recurring series are listed by when the series starts.
