# 0034. Sender rules act on the server only for mail they're sure about

- **Status:** Accepted
- **Date:** 2026-10-25

## Context

The sender rules in 0024 act on the server without asking, after every sync.
Three cases went wrong:

- A **Not spam** rule trusts an address, but From can say anything. Spoofed
  mail using a trusted address (a bank, a colleague), which the server had
  rightly filtered, was pulled into the Inbox.
- **Blocked invitations** were removed from the calendar whenever their UID
  appeared in mail from a blocked sender. Calendar notification services and
  mailing lists relay invitations from people the user does want to hear
  from, so blocking one of them deleted real meetings from the server.
- While another op was retrying, each sync showed the server's state again,
  and enforcement queued the same move once per poll.

## Decision

- Mail comes out of Junk under a Not spam rule only once its body is
  downloaded and it doesn't fail the receiving server's SPF, DKIM or DMARC
  checks (`satchel_mime::auth_failure`, the same test the phishing warning
  uses). Mail that fails stays in Junk; mail not yet downloaded waits for a
  later sync.
- The calendar pass after each sync (`remove_blocked_invites`) removes only
  events the blocked sender organizes. Marking a message as spam still
  removes that message's own invitation, whoever organizes it: the user
  pointed at it.
- The account loop skips enforcement while the account has ops waiting, and
  catches up once the queue is empty.

## Consequences

- A trusted sender whose domain fails authentication (a misconfigured mailing
  list) stays in Junk; the user moves it by hand.
- On servers that add no Authentication-Results header, a Not spam rule works
  as before, since no check fails.
