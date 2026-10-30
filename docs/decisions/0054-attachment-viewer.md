# 0054. View attachments in the app; Open and Save beside it

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
Clicking an attachment only downloaded it: a save dialog on desktop, a
download in the browser. Glancing at a PDF or a photo meant saving it
somewhere first. We wanted a choice between viewing a file in Satchel and
saving it or opening it in its own app.

GolemDocs has its own PDF reader and DOCX layout, but it draws pages through
its editor's layout and WebGL text pipeline. Bringing that over would copy
tens of thousands of lines into Satchel for something the webview can mostly
already do.

## Decision
- Clicking an attachment opens a full-window viewer (`views/mail/attachments.ts`)
  with ←/→ between the message's attachments, **Open** (desktop) and **Save**.
  Right-clicking an attachment offers View, Open and Save.
- The viewer shows only what the webview draws itself: images, PDFs (the
  platform's PDF viewer: WebKit's on macOS, Chromium's in the browser), audio
  and video, CSV/TSV as a table, and text. Anything else shows its name and
  size with Open and Save. `lib/files.ts` decides the kind.
- Files are shown from `blob:` URLs whose type we set to the kind we chose
  (`application/pdf`, `image/*`, `audio/*`, `video/*`), never the sender's
  claimed type. Text and CSV are decoded and inserted as text, never as markup.
  HTML and the other risky attachments are never shown, whatever their type.
  The CSPs allow `media-src blob:` (and `img-src blob:` on the web) for this.
- **Open** is a desktop host command, `open_file`: it writes the attachment to
  its own folder under the app cache, marks it as downloaded (quarantine / Mark
  of the Web, as `save_file` does), and hands it to the OS. Folders older than
  a day are cleared on the next open. Risky files still ask first.
- Office documents (DOCX, XLSX, ODT) are not drawn in the app for now.

## Consequences
- PDFs and images, the bulk of attachments, open without leaving the app or
  leaving copies behind. No new dependencies.
- What a PDF looks like depends on the platform's viewer; the PDF iframe can't
  be sandboxed (Chromium refuses to draw PDFs in sandboxed frames), so we rely
  on the forced type.
- If we want Office documents in the viewer, GolemDocs' DOCX reader and layout
  are the place to start; that would need a decision about sharing its
  packages.
