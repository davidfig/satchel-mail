# 0043. Convert HTML mail to text ourselves

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

HTML-only mail is turned into text for snippets, search and the AI folder.
We used html2text, a renderer built for terminals, and it fit mail badly:

- It lays tables out as columns. Most newsletters are built from layout
  tables, so their text came out chopped into narrow strips
  ("https://tr (https://track.…").
- It panicked on untrusted input. Its list width estimate measures our "• "
  bullet in columns (2) but subtracts its length in bytes (4), so a list
  holding only whitespace underflowed. The panic killed the account's sync
  task without an error, and every restart hit the same message: sync sat at
  "Downloading messages 0/300" and stopped fetching new mail.
- We switched off most of what it does (borders, wrapping, link footnotes)
  and then post-processed its output to undo the rest.

## Decision

`html_to_text` (`crates/mime/src/html_text.rs`) is ours: one pass over the
tokens of html5ever's tokenizer, which we now depend on directly at the
version ammonia already pulls in. Blocks and table rows start new lines and
cells in a row are spaced apart; paragraphs, headings, tables, quotes and
outer lists are set off by blank lines; `<br>` ends a line, or adds an empty
one when the line is empty, as browsers do. Links read `text (url)` for web
and mail addresses, with no repeat when the text is the address and nothing
for image-only links. Lists get "• " or numbers, quotes get "> ", `<pre>`
keeps its spacing. Script, style, title and similar contents are read as raw
text and skipped. Invisible padding characters are dropped.

html2text is removed. Separately, an account's sync loop now runs under a
supervisor: if it panics, the account shows an error and the loop restarts a
minute later, instead of stopping silently.

## Consequences

- Newsletter text reads top to bottom, and conversion can't panic on markup.
- Layout is simpler than a browser's: no CSS (hidden preheaders still show,
  side-by-side cells are joined with spaces), no wrapping. That suits
  snippets, search and agents, which is all this text is for; the reader
  shows the sanitized HTML.
- Snippets and search text for mail already stored keep the old rendering
  until the message is parsed again.
