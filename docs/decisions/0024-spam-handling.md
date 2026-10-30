# 0024. Spam: sender rules outrank the server, and the core warns about phishing

- **Status:** Accepted (enforcement narrowed by 0034)
- **Date:** 2026-10-24

## Context

Every provider runs its own spam filter, and we can't change how it scores
mail. Moving a message to Junk (or out of it) trains some filters, but not
reliably or quickly, so the same sender keeps landing in the wrong place.
The fixtures in `scripts/seed.py` show the cases we care about: spam the
server caught, spam that slipped into the inbox (a display-name spoof, an
HTML "remittance" attachment, an unsolicited invite), marketing that is
better unsubscribed from than reported, and a real message from a contact
filed as spam.

## Decision

**Sender rules.** Mark as spam and Not spam record a standing verdict for
each sender address (`sender_rules`, one row per lowercased address, global
across accounts). Both also move the selected messages and every other
misplaced message from those senders. After every sync the engine runs
`enforce_sender_rules`: spam senders still in the inbox go to Junk, not-spam
senders in Junk go back to the inbox. The moves go through the normal op
queue, so the server (and its filter) sees them. Only the inbox and Junk are
touched; mail you archived or filed stays put. The user's own addresses
never get a rule. Rules are listed and removed under Settings → Privacy.
Rules are by exact address, not domain: blocking a domain is too blunt for
shared providers, and spammers rotate domains faster than addresses anyway.

**Warnings.** `MessageDetail.warnings` is computed in the core when a
message is read, so the UI, CLI and agents all see the same signals:

- the receiving server's Authentication-Results reports a DMARC failure, or
  SPF fail/softfail without a valid DKIM signature. Only failures are used.
  A sender can forge a "pass" header when the receiver adds none, so a pass
  proves nothing.
- the display name matches someone the user has written to (or the user
  themselves) but the address doesn't, or the name is itself a different
  email address. "Known" means you sent them mail, replied to them, or
  marked them Not spam. Receiving mail from an address doesn't make it known.
- Reply-To points at a different domain than From, to an unknown address.
- a link's text shows one domain and its href goes to another. Links
  through a known newsletter click tracker (SendGrid, Mailchimp, …;
  `crates/mime/src/trackers.rs`) don't count: the tracker hides the
  destination rather than naming a wrong one, and flagging them put the
  warning on nearly every newsletter.
- an attachment can run code or imitate a sign-in page (HTML, executables,
  macro documents, disk images).
- an invitation comes from an organizer the user has never written to.

**Unsubscribe.** `Unsubscribe` uses RFC 8058 one-click (an https POST) when
offered, otherwise sends the `mailto:` request from the account, otherwise
returns the https page for the client to open. http-only links are ignored.
The one-click POST runs in the host's `ProviderFactory`, because the core has
no HTTP client.

**Spam invitations.** Many servers put an emailed invitation on the
calendar as soon as it arrives. Mark as spam removes those events, and so
does every calendar sync for blocked senders' invitations from the last 60
days, so one the server adds later comes off too. Only events the user never
answered and doesn't organize are removed. The delete never notifies the
organizer: CalDAV sends `Schedule-Reply: F` (RFC 6638 §8.1, otherwise the
server sends a decline), Google uses `sendUpdates=none`, and Graph only
replies through accept/decline, so deleting as an attendee is already silent.

**In Spam,** links ask before opening, invitations can't be answered (even a
decline confirms the address), and Unsubscribe asks first and warns that it
confirms the address to real spammers.

Agents can read warnings but can't mark spam or change rules: these are
user commands (`Command::allowed_for_agent` is unchanged).

## Consequences

- A wrong verdict is sticky until the user removes the rule. The toast says
  so, and Settings lists every rule.
- Warnings are heuristics. The registrable-domain check is approximate
  (last two labels, three for short country second levels), and display-name
  matching needs names of three or more characters.
- The correspondents list is rebuilt at most once a minute, so a message you
  just sent can take a minute to make its recipient "known".
- If a rule's move fails permanently, the resync puts the message back and
  the next sync tries again. Watch the Activity log for repeated failures.
- That the organizer isn't notified depends on each server honoring
  `Schedule-Reply: F` or `sendUpdates=none`. This has been checked against the
  deletes we send, not against an organizer's mailbox.
- Not done yet: per-domain rules.
