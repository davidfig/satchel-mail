# 0062. Documents and web pages through golemdocs' sandboxed viewer

- **Status:** Accepted
- **Date:** 2026-11-02
- **Partly supersedes:** [0054](0054-attachment-viewer.md) (which files the viewer shows)

## Context
The attachment viewer (0054) shows only what the webview draws itself:
images, PDFs, audio and video, text and CSV. Word and OpenDocument files, the
most common attachments after PDFs and photos, show only a name and Open/Save.
HTML, MHTML and SVG attachments aren't shown at all, because the webview would
run them: they are how phishing pages and script lures arrive.

golemdocs (same owner, AGPL like Satchel) now has an embeddable read-only
viewer: its own readers for Word (.docx, .doc), OpenDocument, RTF, EPUB,
Markdown and a dozen markup formats, HTML, MHTML and SVG, laid out by its
layout worker and drawn with WebGL from MSDF font atlases. It runs in a
sandboxed frame, takes the file by `postMessage`, and fetches nothing but its
own code and fonts (golemdocs `docs/architecture/viewer.md`). 0054 already
named golemdocs as the place to start for Office files.

## Decision
- **The kinds.** `lib/files.ts` gains a `document` kind: Word (.docx, .dotx,
  .doc, .dot), OpenDocument text (.odt, .fodt), RTF, EPUB, Jupyter notebooks,
  Markdown, LaTeX, AsciiDoc, reStructuredText, Org, MediaWiki, Textile, Typst,
  DocBook, JATS, FictionBook and OPML, by extension or, for a file named
  without one, by its type (the name sent to golemdocs gets the extension it
  needs). Markdown moves from `text` to `document`; SVG from `image`.
- **Formerly refused, now shown: HTML (.html, .htm, .xhtml), MHTML (.mht,
  .mhtml) and SVG (.svg, .svgz),** even though the core flags them `risky`.
  They are risky because a browser runs them. golemdocs doesn't: it reads them
  into its own document model and draws that, so their scripts never run,
  their forms and event handlers are dropped, and their remote images,
  stylesheets and frames are never fetched (an image shows its alt text). Only
  these extensions get through: a risky file is shown only when its *name*
  makes it a web page, so `invoice.pdf` sent as `text/html`, a page with no
  extension, `x.html.` and every other risky kind (programs, macro-enabled
  Office files such as .docm, archives, disk images) are still refused, as
  before. Open and Save still ask first, and the chip still warns.
- **Vendored build, not source or a package.** `scripts/vendor-golemdocs`
  clones golemdocs, checks out the commit pinned in `ui/src/lib/golemdocs.ts`,
  runs its `build:viewer`, and copies the result into
  `ui/public/golemdocs/<first 12 of the commit>/`. Copying golemdocs' source
  would fork tens of thousands of lines; an npm dependency would pull its
  whole toolchain into Satchel's build. The pinned build is a reviewable,
  reproducible artifact (rebuilding the same commit gives byte-identical
  files), and the folder name changes with the build, so its files can be
  cached forever.
- **Committed, not fetched at build time.** Building Satchel needs neither
  golemdocs nor network access beyond the usual crates and npm packages, and
  a release is exactly what was reviewed. The cost is repository size: 38 MB
  in 100 files (4.4 MB of code, 33.3 MB of fonts). Git stores files by
  content, so a later build whose fonts haven't changed adds only its code:
  each re-pin so far added between 0.1 and 2.8 MB, the most when the code
  chunks are renamed, which makes every code file new.
- **The pin:** golemdocs `1efae91a0b84d7332c84525f1e4a077df969e8c8`
  (https://github.com/davidfig/golemdocs/commit/1efae91a0b84d7332c84525f1e4a077df969e8c8,
  after its 0.33.0 release): SVG and SVGZ in WebKit's sandbox, and the
  `golemdocs:key` message, which passes ←/→ on whenever the pages fit
  sideways, a page zoomed to fit included.
- **The font set** is golemdocs' default for the viewer: the core atlases
  (Latin, Greek, Cyrillic) of Calibri, Cambria, Arial, Times New Roman and
  Courier New, 33.3 MB. Other fonts are drawn in a stand-in (Georgia in Times
  New Roman, Segoe UI in Arial, Consolas in Courier New) or in Calibri; CJK
  text and characters outside the core atlases are drawn as missing. Every font
  golemdocs has is 174 MB, too much to ship; Calibri alone (7 MB) loses too much.
  `VIEWER_FONTS` passed to the script changes the set.
- **Loaded lazily, in a sandbox, offline.** Nothing of golemdocs loads until a
  document is shown. Then `lib/docviewer.ts` makes an
  `<iframe sandbox="allow-scripts">` (never `allow-same-origin`, so it has an
  opaque origin: a parser bug in untrusted mail can't reach the app's DOM,
  storage, session token or the desktop's IPC), waits for its ready message,
  checking `event.source` against the frame's window, and transfers a copy of
  the bytes. Links go to Satchel's ask-before-opening dialog, always (links in
  documents are a phishing staple); the frame follows none itself. Errors show
  the usual "can't show" panel with Open and Save. Moving to another
  attachment or closing removes the frame, which ends its worker and frees its
  GPU memory. The theme goes with the file.
- **Served with their own headers, on both hosts** (`satchel_core::doc_viewer`):
  - every file but the page: `Access-Control-Allow-Origin: *` (the frame's
    scripts, worker and fonts are CORS requests from origin `null`), its exact
    type, `nosniff`, and a year's immutable cache;
  - `viewer.html`:
    `default-src 'none'; script-src O; worker-src O data:; connect-src O; style-src O; img-src O blob: data:; font-src O; base-uri 'none'; form-action 'none'`,
    with `O` the viewer's origin spelled out (WebKit matches `'self'` to
    nothing in an opaque frame) and `data:` workers (the only kind an opaque
    origin can start). Nothing remote is allowed, so the viewer can never
    reach golemdocs.com or a sender's server.
  - **Desktop:** the app's own `tauri:` scheme can't set headers per file, so
    the files are served from a `golemdocs:` URI scheme
    (`apps/desktop/src/doc_viewer.rs`; `golemdocs://localhost`, or
    `http://golemdocs.localhost` on Windows), read from the embedded UI
    assets (from `ui/public` in a dev build). The app's CSP adds that origin
    to `frame-src`, and the navigation guard lets the frame load it.
  - **Web (`satchel serve`):** `/golemdocs/` is served from the UI folder
    with those headers, plus `frame-ancestors` and `X-Frame-Options:
    SAMEORIGIN` on the page so only Satchel can frame it; a missing file is a
    404, not the app. The app's own CSP already allows a same-origin frame.
    The Vite dev server does the same in `scripts/run dev`.
- **PDFs stay on the platform viewer** for now. golemdocs draws PDFs too, and
  in a sandbox, which the platform viewer can't be (0054); switching would
  also stop Chromium's viewer sending link clicks to the whole tab. Worth
  doing once its PDF view can search.

## Consequences
- Word, OpenDocument, Markdown and web-page attachments show in the reading
  pane, drawn the same on every platform, with nothing fetched and no script
  run. Copy and find work inside the document.
- The app is about 38 MB larger (the desktop binary embeds the files); an
  update to the viewer is a new vendored build, reviewed like any change.
  The pin is reviewed before each release, the first step of the release
  checklist ([RELEASING.md](../RELEASING.md)); it is never updated
  automatically.
- What the viewer can show is golemdocs': a format it reads badly, Satchel
  shows badly. Fixes land in golemdocs, then here through the script.
- Keys pressed in the document that the viewer has no use for come back to
  the viewer pane as `golemdocs:key` messages (Escape with no find bar open,
  letters such as `f`, ←/→ unless the page is zoomed wider than the view),
  so Escape, `f` and ←/→ work as they do with focus on the pane; the viewer keeps its own (find,
  zoom, select all, copy, scrolling). They are taken only from the frame's
  window and only while it has the focus, and only the pane's own keys act
  on them, never the app's mail shortcuts: a compromised frame can at most
  close the viewer or move to another attachment.
- Spreadsheets and presentations (.xlsx, .pptx) are still Open/Save only.
