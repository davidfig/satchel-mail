# 0060. Full Google verification, with a yearly CASA assessment

- **Status:** Accepted
- **Date:** 2026-10-31

## Context
Gmail sign-in asks for `https://mail.google.com/`, a restricted scope (0017).
Until Google verifies the app, only 100 listed test users can sign in, and
they see an "unverified app" warning. Verification for a restricted scope
ends in a CASA security assessment by a lab Google approves, redone every
year. Labs charge for it: about $540 a year at TAC Security, Google's
preferred lab, as reported by blogs rather than Google, and more at higher
tiers. We found no exemption for open-source apps.

The full scope is needed because permanent delete (`messages.batchDelete`,
the Expunge op) only works with it. `gmail.modify` is also restricted, so a
narrower scope wouldn't avoid the assessment.

Alternatives considered:
- **Calendar and contacts only through Google sign-in, Gmail mail over IMAP
  with an app password.** Free and no assessment, but every Gmail user has
  to turn on 2-step verification and create an app password, and some
  Workspace admins block app passwords.
- **Users bring their own client ID.** Free, but about 10 minutes of Cloud
  Console setup, so only for technical users.
- **A verified middleman (Nylas and the like).** Mail would pass through
  their servers, which breaks local-first.

## Decision
Go through Google's full verification with the scopes as they are, and pay
for the CASA assessment every year. A donation page on the site helps cover
the cost.

## Consequences
- Gmail users get one-click sign-in for mail, calendar and contacts.
- A yearly cost and a yearly reassessment. Put the renewal date in the
  release calendar. The tier, and so the price, can go up as the user count
  grows, and a hosted version that stores Gmail data on servers would likely
  raise it.
- Adding another restricted scope later means another review.
- Until verification finishes, the README keeps saying Google sign-in is
  limited to test users.
