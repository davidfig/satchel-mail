# 0072. Send-as addresses, typed in per account

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

People read mail for several addresses in one mailbox: a domain's mail
forwarded to Gmail (Cloudflare Email Routing, say), or aliases on their own
server. Replies should go out from the address the mail was sent to, not the
mailbox's own address. Until now every message's From was the account's
email.

Sending from another address only works if something on the way is allowed
to send for it. With strict DMARC (`p=reject; aspf=s; adkim=s`) a message
from `hello@example.com` sent through `smtp.gmail.com` is rejected, so an
alias often needs its own SMTP server (for Cloudflare Email Sending:
`smtp.mx.cloudflare.net`, port 465, implicit TLS, an API token as password).

Gmail can list an account's "Send mail as" addresses, but only with the
`gmail.settings.basic` scope, and adding a scope reopens Google's restricted
scope verification (0060).

## Decision

**A per-account list of send-as addresses, typed in by hand.** Each has an
email and a display name, stored on the account (`accounts.send_as_json`)
and set with `Command::SetSendAs`. Satchel never fetches them from the
provider, so no new OAuth scope.

**Choosing From.** `Draft.from` names the address (None = the account's
own). The core only builds a message whose From is the account's address or
one of its send-as addresses; anything else is refused, so neither a stale
draft nor an agent's proposal can invent a sender. The composer's From
picker lists every account's address followed by its send-as addresses.

**Replies default to the address the mail was sent to:** the first send-as
address found in the original's To, then Cc, then its `Delivered-To` /
`X-Original-To` headers; else the account's address. Replying to your own
message keeps the From you used. Send-as addresses also count as "me" when
working out reply-all recipients.

**Agents get the same defaults.** A `reply` in the AI folder and MCP's
`propose_reply` default From the same way (the folder looks at To and Cc
only, since it reads the store and not the raw message). Folder `reply` and
`draft` files and MCP's `propose_draft` take an optional `from`, which
creating the proposal checks like any other draft's.

**Per provider:**

- **IMAP/SMTP:** a send-as address may have its own SMTP server (host, port,
  security, username, password). Its password is a separate entry in the
  secret store (`send_as:<account id>:<email>`) inside the single keychain
  item (0052). Without one, the account's SMTP sends it. Either way the
  envelope sender is the send-as address, and the copy goes to the account's
  Sent folder. The core resolves the server and password when the send runs,
  so fixing a wrong password and retrying works. New or changed servers are
  checked (connect and sign in) before they're saved.
- **Gmail:** the message goes through `messages/send` with the alias in
  From. Gmail sends it, or relays it through the alias's own SMTP server, only
  if the address is set up and verified under "Send mail as" in Gmail's
  settings. Otherwise Gmail is documented to replace From with the account's
  address rather than fail, so after sending Satchel reads back the sent
  copy's From and warns if Gmail changed it. *Not yet verified against a live
  Gmail account with a Cloudflare-relayed alias.*
- **JMAP:** the submission uses the identity whose email matches. A send-as
  address with no matching identity is refused with a message saying to add
  it in the provider's settings (the account's own address still falls back
  to the first identity, as before).
- **Microsoft 365 (Graph):** no send-as addresses. Sending as another address
  through Graph needs Exchange permissions (Send As on another mailbox, or
  the tenant-wide "send from aliases" setting) that Satchel can't see or set,
  and when they're missing Graph changes or rejects the message
  inconsistently. The settings section and picker are hidden for these
  accounts, and the core refuses them.

## Consequences

- Users must type their aliases in, and for Gmail also set them up in
  Gmail. The settings help says so.
- A typo'd alias in settings sends nothing wrong: the core only lets the
  user pick listed addresses, and the provider still decides.
- If Gmail rejects or rewrites an alias, the user sees a warning; the message
  has still gone out from the account's address.
- Removing an account or a send-as address deletes its stored SMTP password.
- If Google verification later covers `gmail.settings.basic`, importing the
  list from Gmail can be added without changing how sending works.
