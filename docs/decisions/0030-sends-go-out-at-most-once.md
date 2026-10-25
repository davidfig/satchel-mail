# 0030. Sends go out at most once

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

Sending was an ordinary op: retried on any transient error, and marked done
only after the follow-up work (removing the draft, flagging the original as
answered, a second network call, the audit row). An error in any of those, a
quit, or an account restart in that window replayed the send, so the mail
went out again, up to eight times. A dropped connection after the server took
the message looked the same as one that never connected. A permanent failure
of any op also wiped every sync cursor for the account (0008), including the
`draft:` cursors that say which server copy a draft replaces.

A duplicate email can't be taken back. A send that has to be retried by hand
can.

## Decision

- Before a send starts, its op is marked `sending`. The account loop is the
  only thing that runs ops, so a send still marked `sending` when the loop
  next runs was cut off. It fails with "may have gone out; check Sent" and is
  never retried on its own. The user can retry it from Activity.
- The op is marked done as soon as the provider accepts the message. The
  follow-up work is best-effort and only logged if it fails.
- Providers return `MaybeSent` when a send fails after the message may have
  reached the server: no SMTP reply after the message body, or a dropped
  HTTP or JMAP submission. A refusal the server answered, or a connection
  that never opened, stays retryable. SMTP connects and signs in before
  sending, so being offline is still a plain retry.
- A failed op resyncs only what it touched (refines 0008): mail cursors for
  mail ops, calendar sync tokens for calendar ops, address-book tokens for
  contact ops. Draft cursors are kept. Gmail's history id is kept too: its
  full sync only adds missing messages, so dropping the id fixed nothing and
  lost the changes made since.
- Errors are no longer transient by default. I/O errors retry; IMAP NO/BAD
  replies, "already exists" or "doesn't exist", and JMAP method errors other
  than server failures fail at once instead of holding up the queue.
- Gmail and Graph take recipients from the message headers, so their sent
  message carries a Bcc header; they strip it from the copies other people
  receive. SMTP and JMAP deliver to an envelope and never include it.

## Consequences

- An interrupted send needs a look at Sent and a manual retry, instead of
  possibly arriving twice.
- Other ops that aren't idempotent (creating a calendar event, saving a
  server draft) can still repeat after a crash. They duplicate quietly rather
  than reaching other people, so they're left for later.
