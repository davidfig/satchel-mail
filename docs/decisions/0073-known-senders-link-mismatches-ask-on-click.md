# 0073. A known sender's link mismatch asks on click, not with a banner

- **Status:** Accepted
- **Date:** 2026-11-08

## Context

0067 dropped the "This might be a scam" banner for verified senders and kept
it for everyone else. Lots of real mail from people you write to isn't
verified: small businesses with no DKIM, or no trusted server and a DKIM
check that hasn't run. Their signatures often come from services such as
Newoldstamp. Those link the company's site through the service's own host,
so a thread you've been replying to for weeks still says it might be a scam.

## Decision

A sender counts as known if you've written to them, replied to them, or
marked their mail Not spam. That is the same test the impersonation and
Reply-To checks use. A known sender's mismatched links get no banner, only
the click prompt from 0067, unless the message failed authentication.

## Consequences

- Mail from people you correspond with no longer looks like a scam when it's
  opened. Their mismatched links still ask when they're clicked.
- Someone forging a known address on a domain with no SPF, DKIM or DMARC
  doesn't get the banner. Failing authentication brings it back, and the
  click prompt still shows either way.
