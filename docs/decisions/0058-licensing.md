# 0058. Licensing: AGPL app, MIT/Apache libraries, a CLA

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

satchel.email will be open sourced. Selling a desktop mail client is a hard
market to enter, and an app that reads someone's whole mailbox and lets
agents in is far easier to trust when its code can be read. The repository
stays private until that's done, so nothing here has any effect before then.

A hosted version (0018) may still come, and some crates are useful outside
the app.

## Decision

- **App: AGPL-3.0-only.** The workspace default, covering `core`,
  `providers`, `mcp`, `cli`, `apps/desktop`, `ui` and everything else. AGPL
  rather than GPL because `satchel serve` and a future hosted version run
  over a network: anyone hosting a modified copy must publish their changes.
- **Libraries: MIT OR Apache-2.0**, the Rust convention, for `mime`, `ical`
  and `api`, which depend on nothing else in the workspace. Each has
  `LICENSE-MIT` and `LICENSE-APACHE`, and its `.rs` files carry an
  `SPDX-License-Identifier` line because they are the parts expected to be
  copied or used on their own. App files have no per-file headers.
- **Contributions under a CLA** (`CLA.md`): contributors keep their
  copyright and license it to the project under any terms, so a hosted
  version, a commercial license or a later relicense stay possible.
- **History kept** as it is; the secrets scan of 2026-10-31 found nothing.
- **Names are separate:** `TRADEMARKS.md` keeps forks from shipping as
  satchel.email (0055).

## Consequences

- Releases carry `LICENSE` and every dependency's license text
  (`THIRD_PARTY_LICENSES.txt`, built by `scripts/third-party-licenses.py`
  in `scripts/release.sh`).
- CLA signing is `.github/workflows/cla.yml`, our own workflow on GitHub's
  `actions/github-script`: the author comments a fixed sentence once and the
  workflow sets a `CLA` commit status. The CLA Assistant action is archived
  and cla-assistant.io needs an OAuth grant and is barely maintained.
- When the repository goes public: turn on private vulnerability reporting
  (`SECURITY.md` points to it) and make `CLA` a required status check.
- Publishing `satchel-mime`, `satchel-ical` or `satchel-api` to crates.io is
  now allowed by their licenses, but not yet planned.
- Open source doesn't remove Gmail's restricted-scope verification (0017): a
  fork needs its own OAuth client ids, or uses IMAP or JMAP.
- Some companies avoid AGPL code; the libraries are permissive so that
  doesn't stop them using those.
