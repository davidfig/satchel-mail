# 0076. The AI folder lists every message's metadata, for all history

- **Status:** Accepted
- **Date:** 2026-11-10
- **Amends:** 0022 (what folder agents can see)

## Context

The AI folder writes `thread.md` only for its date limit (a year by
default), and its indexes list only the newest 200 threads. In the bake-off
(see 0075), the folder's clear win was bulk work over metadata: one `jq`
pass over `meta.json` did what took MCP more than 100 calls. Its clear loss
was anything older than the window. "Every PowerPoint I've received" was
impossible through the folder, and over MCP the agent gave up after 2,622
calls without trusting the search.

Most bulk questions (top senders, what had a PDF, what's unanswered,
spending from order emails) need sender, subject, flags and attachment
names, not message bodies.

## Decision

- **`mail/<account>/history/<yyyy-mm>.jsonl`:** one line per message, every
  month the account has mail in. Each line has `date`, `message_id`,
  `thread_id`, `from` (`name`, `email`), `from_me` (the account's address or
  a send-as address), `to`, `cc` (addresses), `subject`, `mailboxes`,
  `unread`, `flagged`, `answered`, `draft`, `attachments` (`name`, `type`,
  `size`, not inline images), and the snippet cleaned as in 0075.
- **The folder is two layers:** metadata for all history, readable text
  (`thread.md`) for the date limit. The README says so and shows `jq`
  examples. A message's thread folder is found from its `thread_id`'s last
  8 characters.
- **On by default** (`history_all`). Turning it off limits history to the
  same dates as the threads. This widens what folder agents see (0022): who
  the user has written to and heard from, and about what, since their
  oldest mail, but not message bodies. MCP agents with an "Everything"
  policy already see that much.
- **Proposals from folder files are still limited to the date limit**, as
  before. Acting on older mail is a separate decision.
- **Written by month, only when a month changes.** Each pass fingerprints
  each month from its messages' dates, versions (which change with flags,
  mailboxes and content) and thread ids, plus the account's mailbox names
  and addresses. A month is rebuilt only when its fingerprint changes.
  Fingerprints live in memory, so the first pass after a start compares
  every file (writing only those that differ). Months with no messages
  left, or outside the dates when history is limited, are removed.

## Consequences

- Questions over all mail become one `cat mail/*/history/*.jsonl | jq …`.
- On a real four-account mailbox (87,939 messages back to 2004): 724 month
  files, 53 MB. Building them from nothing takes about 1 s; a pass where
  nothing changed takes about 50 ms. Finding every PowerPoint ever received
  is one `jq` command, 0.6 s.
- Gmail's AMP bodies (`text/x-amp-html`) look like attachments in the
  message structure and are left out of `attachments`.
- Thread merges don't bump message versions, so they're caught by the
  thread ids in the fingerprint.
- Snippets are only as complete as the stored copy: freed content (0036)
  keeps its snippet.
