# 0081. Reply warnings only where a reply could go astray

- **Status:** Accepted
- **Date:** 2026-11-14

## Context
Two of the message warnings fired on ordinary inbox mail far more often than
on anything dangerous:

- **Reply-To differs from the sender.** Newsletter and transactional mail
  services send from their own domain and point replies at the company, so
  most bulk mail tripped it.
- **Unknown inviter.** Scheduling services (Calendly and the like) send
  invitations from a no-reply address nobody has written to.

Too many false alarms teach people to ignore the banner, including the times
it matters.

## Decision
- A different Reply-To is flagged only when the From address belongs to
  someone you've written to (or marked not spam). That's the case a reply
  can be hijacked: mail that looks like a known person but answers go
  elsewhere. Mail from a stranger has no trusted address to divert from.
  It still shows only in the reply box, not on the message.
- On list or no-reply mail (`bulk`), the unknown-inviter warning moves from
  the message to the reply box, which starts closed for that mail, so it
  shows only once you go to reply. RSVP buttons don't ask first.

## Consequences
Fewer banners in the inbox. A stranger's mail that sends replies to a third
address is no longer flagged; impersonation and authentication warnings still
cover lookalike senders.
