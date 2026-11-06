# 0061. Check DKIM ourselves when no server is trusted

- **Status:** Accepted
- **Date:** 2026-11-02

## Context

Trust decisions (remote images for allowed senders, "not spam" rules, and
skipping the deceptive-link warning when a sender links its own domain)
need the From domain to be verified. Until now only a trusted server's
Authentication-Results counted (0042). Accounts on a server Satchel has no
built-in id for trust nothing until about 20 messages show the same
authserv-id. A server that writes no Authentication-Results never earns
trust. Real mail from a bank could then carry a scam warning.

Lowering the threshold or adding guessed provider ids would weaken a
security decision, or only move the gap. Checking the signature
ourselves closes it for every server.

## Decision

For accounts with no trusted server, the host checks DKIM signatures with
`mail-auth` (Stalwart's library), using the system's DNS resolver
(`ProviderFactory::dkim_pass`). A signature counts only when it:

- verifies,
- covers the From header,
- has no `l=` body length (otherwise text can be added after the signed part),
- and its signing domain is the From domain or a parent of it (the same
  alignment as a `dkim=pass` from a trusted server). The host checks the
  first three, and the core checks alignment.

A message with more than one From header never passes.

The check runs when a message is opened, waits at most 3 seconds, and its
result is stored in `messages.dkim_pass` (schema 8). It runs once, because
signing keys get rotated and a later check could fail. A check that
couldn't run (offline, DNS error) stores nothing, and is tried again the
next time the message is opened. Accounts with a trusted server don't use
it: their server's verdict stands.

## Consequences

- New dependencies: `mail-auth` and `hickory-resolver` (DNS). `mail-auth`
  0.13.3 uses our `mail-parser` and `aws-lc-rs`, and needs no HTTP client.
- Opening a message on such an account looks up the signer's DKIM key,
  so the user's DNS server sees which domains send them mail. It already
  sees the links and images they open.
- Sender rules use the stored result, so "not spam" mail on these
  accounts leaves Junk only after it has been opened once.
- Old mail whose signing key has been retired stays unverified. That is
  safe: it is shown with the usual warnings.
- SPF isn't checked: it needs the sending IP, which only the receiving
  server knows.
