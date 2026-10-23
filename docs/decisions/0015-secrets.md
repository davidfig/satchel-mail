# 0015. Credentials in the OS keychain

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Passwords and OAuth tokens must not sit in the database or the AI folder.

## Decision

Credentials live behind the `SecretStore` trait: the OS keychain
(`app.satchel.mail`) by default, a 0600 JSON file with `SATCHEL_SECRETS=file`
for tests and headless servers. Agent tokens are stored only as BLAKE3
hashes.

## Consequences

- The hosted version needs a KMS-backed `SecretStore`.
