# 0063. Automatic updates from GitHub Releases, offered with a toast

- **Status:** Accepted
- **Date:** 2026-11-03

## Context

Releases are built and signed in CI and published on GitHub Releases (0059),
but an installed app had no way to hear about them: people would have to
notice a new version and download it by hand. The Tauri updater plugin was
already a dependency behind the `updater` feature, and `scripts/release.sh`
could turn it on, but nothing built a feed or used it.

## Decision

- **Feed:** a static `latest.json` in each GitHub release, made by the
  release workflow from that release's files. Apps read
  `https://github.com/davidfig/satchel-mail/releases/latest/download/latest.json`,
  so a release reaches installed apps when its draft is **published**, never
  before. We host nothing and keep no server.
- **What updates:** the Mac app (an `.app.tar.gz` per architecture), the
  Windows installer and the AppImage. `.deb` installs aren't in the feed; they
  update through the package manager or a new download.
- **Signatures:** every update bundle is signed with the Tauri update key
  (minisign), on top of the platform signing in 0059. The public half is
  built into the app; the app rejects a bundle the key didn't sign, so a
  compromised download host can't push code. The private key is a repository
  secret, and a `v*` tag fails without it, like the other signing keys.
- **When:** the app checks at start and every six hours, downloads a newer
  version in the background, then shows a toast that stays until dismissed:
  "satchel.email X is ready to install — Restart to update". Nothing
  installs without that click: restarting closes the window and stops syncs,
  so the user picks the moment. Dismissed, it isn't offered again until the
  app restarts or a newer version comes out.
- **Version check:** a tag must match `version` in `tauri.conf.json`, or the
  feed would announce a version the app never becomes and offer it forever.
- The host owns this (`apps/desktop/src/update.rs`); it isn't an engine
  `Command`, since `satchel serve` and the CLI don't update themselves.

## Consequences

- Losing the update private key strands every installed copy: they would
  only accept bundles signed by it. Keep a copy outside GitHub (a password
  manager). Rotating it means one release signed by the old key that ships
  the new public key.
- A bad release reaches everyone who clicks Restart. Check the draft before
  publishing; to pull a release, unpublish it (back to draft) so `latest`
  points at the previous one, then ship a fix with a higher version (apps
  never downgrade).
- The Mac builds now also produce the `.app` bundle the update is made from.
- Not covered: release notes in the toast, beta channels, and updating
  `satchel serve` or the standalone CLI.
