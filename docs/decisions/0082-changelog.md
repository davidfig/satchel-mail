# 0082. A hand-written changelog, published as the site's What's New page

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

Help → What's New opened the latest GitHub release, whose notes came from
`gh release create --generate-notes`. That lists merged pull requests, and
most work is committed straight to `main`, so a release's notes were usually
just a compare link. People who clicked What's New learned nothing, and
there was no record anywhere of what each version changed for them. The
commit subjects are good, but they're written for the code (one release
mixes site edits, version bumps and user-facing fixes).

## Decision

**`CHANGELOG.md` at the repository root is the one source.** A section per
release, `## X.Y.Z (YYYY-MM-DD)`, newest first, with an optional paragraph
and a list of what people will notice, written for them. It was backfilled
from 0.1.0 to 0.1.17 from the commit history.

**The site's page is generated from it.** `scripts/changelog.py html` writes
`site/changelog.html` (the site has no build step, so the generated page is
committed). Each release has an anchor, `#v0.1.17`. The page is linked from
every page's footer.

**The app shows it itself.** The UI bundles `CHANGELOG.md` at build time and
Help → What's New and the palette open it in a dialog
(`ui/src/app/changelog.ts`), so it needs no browser and no network. Its
parser reads the same subset as the script. (The first version opened the
site in the browser instead; 0.1.18 shipped that way.)

**Releases use the same text.** The release workflow checks, before the long
build, that the tag has a section and that the page matches the file, and
the draft release takes its notes from that section instead of
`--generate-notes`. The update feed's `notes` field carries the section's
text, so the app can show a downloaded version's notes before it's
installed, when its own bundled changelog doesn't have them yet (the
`update_notes` host command).

The script understands only what the file uses (headings, paragraphs, `- `
lists, `code`, bold, links) and is a few dozen lines of Python with no
dependency, like `scripts/third-party-licenses.py`.

## Consequences

- Writing the section is a release step (docs/RELEASING.md); forgetting it
  fails the tag rather than shipping empty notes.
- The page is a generated file in `site/`: edit `CHANGELOG.md`, not the page,
  and rerun the script. `check` catches a page edited by hand.
- The app's copy is as of its build: a typo fixed in `CHANGELOG.md` after a
  release reaches the site at once and the app with the next release.
- The update toasts open the dialog at their version. "Ready to install" has
  a What's new button beside Restart to update (it leaves the toast up) and
  shows the new version's notes from the feed above the bundled ones; the
  first start on a new version says "Updated to satchel.email X" with the
  same button. The UI remembers the last version it ran in `localStorage`,
  so a first install says nothing, and the host's `app_version` command says
  which version is running.
- 0.1.18's feed `notes` is a link, not text; only builds from 0.1.19 on read
  the field, so nothing shows it.
