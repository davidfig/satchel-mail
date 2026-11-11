# 0077. Search finds attachments by kind and name, and MCP says how many match

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

In the bake-off (see 0075), "every PowerPoint I've received" failed over MCP.
Free text already matches attachment names (the FTS index has an
`attachments` column), but nothing said so, and `pptx` as free text also
matches bodies and links. The agent couldn't tell whether a page of results
was all of them, so it kept paging and gave up after 2,622 calls.

## Decision

- **`has:<kind>`:** `pdf`, `document`, `spreadsheet`, `presentation`,
  `image`, `video`, `audio`, `archive`, `calendar` (plurals too), each a list
  of file extensions; or any extension itself (`has:pptx`). A comma list
  matches any (`has:pdf,spreadsheet`). `has:attachment` is unchanged.
- **`filename:<text>`:** an attachment whose name contains the text, with a
  comma list matching any. `%` and `_` are plain characters.
- **What counts as an attachment:** any part except an image the HTML shows
  in place (inline, with a Content-ID: signature logos) and Gmail's AMP body.
  Apple Mail marks attached documents inline, so inline alone doesn't rule
  one out. The AI folder's history files (0076) list attachments by the
  same rule.
- **MCP `search_threads` returns `{ total, threads }`:** the count of every
  matching thread (one more `CountThreads`), so an agent knows when it has
  seen them all. Its description now says free text matches attachment
  names and lists the new operators.

## Consequences

- Agents and people can ask for "presentations from Bob this year" in one
  query, and smart folders can use the operators.
- `has:attachment` still uses the stored flag, which leaves out every inline
  part, so `has:pdf` can find a message `has:attachment` doesn't (an Apple
  Mail PDF). Aligning the flag needs every message's flag recomputed; left
  for later.
- `has:` followed by an unknown word of up to eight letters and digits is
  read as an extension and usually finds nothing, where it used to be free
  text.
- Matching reads each message's `attachments_json`. On a real mailbox
  (87,939 messages), `has:presentation` takes 0.09 s and finds 27 threads;
  `has:pdf` takes 0.07 s and finds 2,400, of which 29 have only inline
  PDFs that `has:attachment` misses.
- `search_threads` returning an object instead of a list changes its shape
  for agents that parsed the list.
