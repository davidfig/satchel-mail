# 0055. Name: satchel.email

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

The product is reached at satchel.email, and the window title already used
that name. Calling it "Satchel Mail" everywhere else meant two names.

## Decision

The product is named satchel.email, written in lower case, everywhere the user
or another program sees a name: the app bundle (`satchel.email.app`), dialogs,
the CLI, MCP instructions, the AI folder README, `X-Mailer` and `PRODID`.

Identifiers that store data keep their old values, so existing installs carry
on: the bundle identifier and keychain service `app.satchel.mail`, the data
directory (`ProjectDirs("app", "Satchel", "Satchel Mail")`), and the
"Satchel Mail Dev" signing certificate name. The repository and crate names
stay `satchel-*`.

## Consequences

- An installed `Satchel Mail.app` is replaced by `satchel.email.app`; remove
  the old copy and any Dock shortcut to it.
- The data directory's folder name no longer matches the product name.
