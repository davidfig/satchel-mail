# 0064. Release Mac and Linux first; Windows joins when it's signed

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

0059 made a `v*` tag fail unless both Mac and Windows were signed. Mac signing
and notarization work. Windows signing depends on Microsoft's identity check
for Artifact Signing, which takes 1 to 20 business days and isn't in our
hands. Holding every platform back for it would delay the first release for
no benefit to Mac and Linux users.

## Decision

- A tag still fails if Mac isn't signed and notarized.
- Until the repository variable `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE`
  is set, a tag skips the Windows build. **No Windows file goes in the release
  or in `latest.json`.** We leave Windows out rather than ship it unsigned, so
  0059's "nothing unsigned by accident" still holds.
- Setting that variable is the switch: from then on a tag builds Windows and
  fails if its signing is incomplete, as 0059 describes. Nothing else changes.
- Linux ships as it does today: not code-signed, with `SHA256SUMS` and
  attestations.
- The site shows Windows as coming soon instead of a download link until the
  first release with Windows is published.

## Consequences

- Windows visitors can't download anything at first, and building from source
  is their only option.
- The update feed has no Windows entry until then. No Windows install exists
  to update, so that loses nothing; the first Windows release adds the entry.
- Someone has to remember the switch: when the identity check completes, set
  the variable, add the login secrets, and restore the site's Windows link
  (`docs/RELEASING.md`).
