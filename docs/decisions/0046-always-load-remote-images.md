# 0046. An opt-in setting loads remote images, except in spam

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
Remote images are blocked by default because they tell senders when, where
and whether you opened their mail. Until now the only way to see them was
per message ("Show once") or per sender ("Always for …"), which is tedious
for people who read a lot of newsletters and don't mind the tracking.

## Decision
Settings → Privacy has "Always load remote images, except in spam"
(`Settings::load_remote_images`, off by default). When on, the core loads
remote images for every message except mail in the account's Junk folder
and mail from a sender the user marked as spam. The sanitizer and the
iframe's CSP are unchanged; only `allow_remote` is decided differently.

Blocking stays the default, as in Apple Mail's Protect Mail Activity,
Thunderbird and Fastmail. Gmail and Outlook load images by default, but
fetch them through their own proxy so the sender doesn't see your IP; we
have no proxy, so loading here reveals it.

## Consequences
- Senders learn your IP address and when you read their mail.
- Spam is still the riskiest mail, so it never auto-loads; "Show once"
  still works there.
- Mail that is forged but not in Junk does load images under this setting,
  unlike the per-sender allowance, which requires a verified From.
