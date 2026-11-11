# 0075. Message text for agents is cleaned of tracking and padding

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

A bake-off on a real mailbox, with agents doing the same tasks through the
AI folder and through MCP (see the vault note "AI Folder vs MCP Bake-off
2026-11-10"), found the largest cost on both sides was noise in message text.
Hotel and restaurant emails came to 18–33 KB of mostly links. Across the
7,340 `thread.md` files in that folder:

- 39% of the bytes were links, nearly all over 100 characters: click
  trackers, Outlook safelinks, LinkedIn's tracking parameters.
- 35% of lines were blank, and 162k invisible padding characters were mixed
  in.
- Some retailers (Costco) send a plain-text part laid out in columns, padded
  with hundreds of spaces, with links chopped into pieces between the
  columns. No cleaning can recover that.

Agents can't use a tracking link, and reading one costs as much as reading
a sentence.

## Decision

`satchel_mime::agent_text` (and `agent_body` for a parsed message) is the
text both agent doors show:

- **Links:** Outlook safelinks are unwrapped to the link they hold.
  Tracking parameters (`utm_*`, `fbclid`, `mc_eid`, LinkedIn's
  `trackingId`/`lipi`/`midToken`, …) are removed; generic names like `id`
  or `e` are kept, because sites use them. A link that still runs over 100
  characters, or goes through a known click tracker (the list from
  `trackers.rs`), is cut to its host: `https://links.shop.example/…`. A link
  a plain-text part hard-wraps over several lines is joined first.
- **Padding:** invisible characters go, non-breaking spaces become spaces,
  runs of three or more spaces inside a line become two, and runs of blank
  lines become one.
- **Column layouts:** when at least a fifth of a plain-text part's lines pad
  columns apart (eight spaces or more) and the message has HTML, the HTML
  part's text is used instead.
- The AI folder's `thread.md` and MCP's `get_thread` show each message's text
  with quoted history removed, then cleaned. MCP's `get_message` is cleaned
  but keeps the quotes. MCP's own simpler quote stripper is replaced by
  `strip_quotes`, so both doors agree.
- Every AI folder thread is rewritten once (setting `ai_folder_text` =
  `agent-v1`) so older files match.

The app's own reading view, replies, forwards and search are unchanged:
people see and send the message as it came.

## Consequences

- Measured on 8,794 stored messages (every tenth, from 2004 on): 27.4 MB →
  15.0 MB of agent text, 45% less. Words other than links: 1,719,132 →
  1,718,735. The few messages that lost more than a tenth of their words
  lost quoted history that the plain text hid and the HTML marked, or a
  marketing email's image captions. On the 7,340 existing `thread.md`
  files: 57.8 MB → 35.9 MB.
- An agent can't follow a shortened link. The folder README says so, and
  that it should ask the user. Exact links are in the message in the app.
- Cleaning takes about 0.1 ms per message (0.75 s for all 7,340 files).
- Snippets aren't changed here. The full history index planned next uses
  this cleaning for its snippets.
