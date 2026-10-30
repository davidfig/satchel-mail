# 0052. All credentials in one keychain item

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

The keychain store (0015) kept one item per account. macOS asks before an app
reads an item it didn't create, and "Always Allow" covers only that item, so a
second copy of the app (the installed release next to a dev build, or a build
after the signing identity changed) asked once per account.

## Decision

`KeychainSecretStore` keeps every credential in a single item, `credentials`
under the `app.satchel.mail` service, holding a JSON object keyed like the
`SecretStore` trait (`account:<id>`). Writes re-read the item first so a key
the CLI added isn't lost. An old per-account item moves into the bundle the
first time it's read, and is then deleted.

## Consequences

- A new copy of the app asks once in total, however many accounts there are.
- Every write rewrites all credentials, so two processes writing at the same
  moment can still lose one write. Only the engine owner and short CLI
  commands write, so this is rare.
- Removing an account rewrites the bundle rather than deleting an item.
