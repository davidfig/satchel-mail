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
every page's footer, and Help → What's New and the palette open
`https://satchel.email/changelog`.

**Releases use the same text.** The release workflow checks, before the long
build, that the tag has a section and that the page matches the file, and
the draft release takes its notes from that section instead of
`--generate-notes`. The update feed's `notes` field points at the page's
anchor for that version.

The script understands only what the file uses (headings, paragraphs, `- `
lists, `code`, bold, links) and is a few dozen lines of Python with no
dependency, like `scripts/third-party-licenses.py`.

## Consequences

- Writing the section is a release step (docs/RELEASING.md); forgetting it
  fails the tag rather than shipping empty notes.
- The page is a generated file in `site/`: edit `CHANGELOG.md`, not the page,
  and rerun the script. `check` catches a page edited by hand.
- What's New now needs the site, not GitHub; GitHub still has the same notes
  on each release.
- The update toasts link to the page too. "Ready to install" has a What's
  new button beside Restart to update (it leaves the toast up), and the first
  start on a new version says "Updated to satchel.email X" with the same
  button. The UI remembers the last version it ran in `localStorage`, so a
  first install says nothing, and the host's `app_version` command says which
  version is running.
- Not covered: showing the notes inside the app itself (a What's New dialog).
  The feed's `notes` link makes that possible later.
