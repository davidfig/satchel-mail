# 0017. OAuth with PKCE and a loopback redirect

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Gmail and Microsoft 365 require OAuth; desktop apps can't keep a secret.

## Decision

Authorization code + PKCE (S256) with a random-port loopback redirect
(RFC 8252). Client ids come from the environment or are compiled in at
release time (`SATCHEL_GOOGLE_CLIENT_ID`, `SATCHEL_MS_CLIENT_ID`, …). Tokens
refresh automatically; a revoked grant surfaces as an auth error.

## Consequences

- Gmail's restricted scope needs Google verification and a yearly security
  assessment before public launch (see docs/OAUTH.md).
