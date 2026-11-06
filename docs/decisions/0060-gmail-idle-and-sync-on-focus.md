# 0060. Gmail new-mail signal over IMAP IDLE, and a check on window focus

- **Status:** Accepted
- **Date:** 2026-11-06

## Context
Gmail and Outlook accounts only polled, every 120 seconds by default, so new
mail took a minute on average and two at worst to appear even though a sync
pass takes seconds. The app also didn't check when you came back to it, so it
could look out of date just when you opened it.

The providers' own push doesn't fit a local-first app: Gmail's needs a Cloud
Pub/Sub topic and Outlook's a public webhook, so either would need a hosted
relay.

## Decision
- Gmail accounts hold an IMAP IDLE connection to `imap.gmail.com` between
  passes, signed in with XOAUTH2 using the `mail.google.com` permission we
  already request. It only signals when to sync: the inbox is opened with
  EXAMINE (read-only) and all mail still comes through the API.
- If the connection fails for any reason (IMAP blocked by a Workspace admin, a
  network that blocks port 993), the account polls as before. A failure never
  shows as an account error; the API sync reports real sign-in problems.
- The IDLE wait still ends at the poll interval, so other changes keep being
  picked up. Each wait compares the inbox's message count with the last one, to
  catch mail that arrived while nothing was listening.
- Outlook stays on polling.
- The UI asks for a sync of all accounts when the window gains focus, at most
  once every 30 seconds, through the normal `sync_now` command.

## Consequences
- Gmail inbox mail appears within seconds. Mail skipping the inbox (filters to a
  label, spam) and Outlook mail still waits for the poll or a focus check.
- Each Gmail account holds one more connection and signs in to IMAP once per
  poll interval. A very short interval means many sign-ins.
- Each arrival runs two short sync passes: the second comes from the changed
  message count.
- Any inbox change, such as archiving or a flag set elsewhere, also wakes the
  loop, so those show up sooner too, at the cost of an extra pass.
- A focus check also syncs address books, which is why it's limited.
