# 0059. Signed releases from CI: Developer ID, Artifact Signing, attestations

- **Status:** Accepted
- **Date:** 2026-10-31

## Context

Releases are built for Mac, Windows and Linux by `.github/workflows/release.yml`
and published on GitHub Releases. Unsigned, the Mac app is blocked by
Gatekeeper and the Windows installer by SmartScreen, so most people would never
get it open. Since 2023, Windows code-signing keys must live in hardware (a
token or HSM), so a `.pfx` in a CI secret is no longer an option. Linux has no
OS-level signing, but people should still be able to check a download came
from this repository.

## Decision

- **Mac:** a Developer ID Application certificate (Apple Developer Program),
  imported into a keychain the workflow owns. Tauri signs the app and its
  bundled `satchel` with hardened runtime and notarizes it; the workflow signs
  and notarizes the standalone CLI the same way, and notarizes and staples the
  DMG around the app. Notarization prefers an App
  Store Connect API key over an Apple ID, so CI doesn't hold your password.
- **Windows:** Azure Artifact Signing (formerly Trusted Signing): Microsoft
  verifies the publisher and holds the key, and it costs a fraction of an OV
  certificate in a cloud HSM. Tauri's `signCommand` runs
  `artifact-signing-cli` on the app, the bundled `satchel.exe` and the
  installer; the workflow signs the standalone CLI.
- **Every file:** a `SHA256SUMS` in each release, and a GitHub build
  attestation (`gh attestation verify`) once the repository is public. Linux
  relies on these.
- **No unsigned releases:** a `v*` tag fails unless the Mac and Windows keys
  are set. Branch builds sign with whatever keys exist and warn about the rest.
- Keys are repository secrets; names that aren't secret (the Artifact Signing
  endpoint, account and profile) are repository variables. Setup steps are in
  `docs/RELEASING.md`.

## Consequences

- Costs: the Apple Developer Program ($99 a year) and Artifact Signing (about
  $10 a month).
- Keychain items are tied to the signing Team ID (0052): moving from the
  Apple Development certificate to a Developer ID may make existing installs
  ask for keychain access once.
- Artifact Signing's identity check has had country limits for individuals;
  if it isn't available, an OV certificate in a cloud HSM drops into the same
  `signCommand`.
- Not covered yet: GPG-signing the AppImage or an apt repository, and the
  Tauri updater's own update signature (`TAURI_SIGNING_PRIVATE_KEY`, already
  supported by `scripts/release.sh`).
