# 0067. A verified sender's link mismatch asks on click, not with a banner

- **Status:** Accepted
- **Date:** 2026-11-05

## Context

The deceptive-link check (0024) put "This might be a scam" on any message
with a link whose text shows another domain. 0024 later exempted verified
senders whose link text shows their own domain, but real mail still trips it:
UnitedHealthcare sends from `benefits.unitedhealthcare.com` with links that
show `myuhc.com` and go to `click.benefits.unitedhealthcare.com`.

Nothing local can tell that those two domains are the same company. Skipping
the warning when a link goes to the sender's own domain isn't safe either: a
phisher can register `paypal-alerts.example`, pass DKIM on it, and show
`paypal.com` on a link to their own site. Verification proves who sent a
message, not that they're the brand they name. WHOIS and RDAP usually hide
the registrant, and looking up owners would tell a third party which domains
send the user mail.

Thunderbird had the same problem. Its banner drew years of false-positive
reports, some users turned scam detection off, and since Thunderbird 68 it
warns only when the mismatched link is clicked.

## Decision

The core lists a message's mismatched links in `MessageDetail.deceptive_links`
(address, shown host, actual host). The address is there because on desktop
WebKit a click reaches us as a bare URL, without the link's text.

- **Verified sender:** no banner. Clicking one of those links asks.
- **Unverified sender:** the banner stays, and clicking also asks.
- **Verified sender showing its own domain:** neither, as before (0024).
- **Links through a known click tracker:** neither, as before (0024).

The prompt says what the link shows and where it goes, and offers Cancel,
Continue anyway, and Go to the shown domain (the primary button).

## Consequences

- The warning appears when someone is about to follow the link, and real
  mail no longer looks like a scam when it's opened.
- A phisher with their own verified domain no longer gets a banner, only the
  prompt. Someone who reads the mail and acts without clicking (calling a
  number, replying) sees no warning for the link. Failed authentication,
  impersonation and Reply-To warnings still show.
- Such a message also isn't `cautious` any more, so its other links open
  without a question.
- Matching is by exact address after URL normalization. If the host reported
  a click with a differently encoded address, it wouldn't match and the link
  would open without the prompt.
- Going to the shown domain opens its home page, not the page the link meant.
- On the web in WebKit, a click goes through the `#follow` page, which asks
  with the full address rather than this prompt.
