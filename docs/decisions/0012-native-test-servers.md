# 0012. Native local test servers, no Docker

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

We need real servers to test sync without touching personal accounts, and the
user preferred not to run Docker.

## Decision

`scripts/dev-servers` downloads and runs Stalwart natively on localhost-only
ports. Stalwart covers IMAP, SMTP, JMAP and CalDAV, so the originally planned
Radicale isn't needed. `init` configures it with stalwart-cli and saves a
template; `reset` restores the template and seeds fixtures (`scripts/seed.py`).

## Consequences

- One binary to manage; resets take seconds.
- Gmail and Graph can't be tested locally.
