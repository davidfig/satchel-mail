# 0044. Approved drafts can be sent straight from Review

- **Status:** Accepted
- **Date:** 2026-10-30
- **Amends:** 0006

## Context

0006 made approving a draft open it in the composer, so the user pressed Send
there. In practice that's two steps for the common case: the reply on the
review card is already what you want, and the card already lets you edit the
To, Subject and text. The "Approve & open draft" button also didn't say
clearly what it did.

## Decision

A draft proposal's card has three buttons: **Send** (primary), **Save as
draft** (the old behaviour: save to Drafts and open the composer) and
**Reject**.

Send is still a human action: the UI calls `approve_proposal`, which saves the
draft, then `send` as the user, through the same undo window as the composer.
Undo or a failed send leaves the message in Drafts and opens it. Nothing
changes for agents: they still can't send, and `send` still refuses any actor
but the user.

What's sent is what the card shows. The card shows the plain text only (an
agent's `body_html` is dropped, as before), lists Cc, Bcc and attachments,
and the existing risk warnings (outside recipients, Bcc, forwarded
attachments) and staleness check apply before sending.

## Consequences

- Replying to an agent's draft is one click instead of two.
- A careless click now sends mail rather than opening a composer; the undo
  window and the warnings are the safety net. Watch for whether risky drafts
  (with warnings) need Send demoted or a confirmation.
