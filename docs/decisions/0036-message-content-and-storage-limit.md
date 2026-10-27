# 0036. Download all mail without attachments, within a storage limit

- **Status:** Accepted
- **Date:** 2026-10-26

## Context

Sync downloaded full content only for the last `initial_sync_days` (90 by
default). Older mail had headers and a preview, and its content came down
only when opened (0005). That kept the first sync fast, but the AI folder,
the main agent interface (0022), wrote the one-line preview for every older
message, without saying anything was missing. Agents searching
`~/Satchel/` couldn't find anything in older mail. Full-text search had the
same gap.

Downloading everything has two costs. Each raw message includes its
attachments, so a mailbox's full size ends up on disk. And some people don't
have the disk space for that.

## Decision

- **All mail's content downloads in the background.** After the recent
  window, the account loop keeps going newest first, a batch per sync (300
  on the first pass, 100 after). The new `download_days` setting can stop it
  at a year or five years. The default is all mail. `initial_sync_days` now
  only decides what comes first.
- **Attachments download only when asked for.** Stored copies leave out
  attachment contents (`satchel_mime::strip_attachments`). Each attachment
  part keeps its headers and loses its body, so the attachment list and ids
  stay the same. The message is marked `partial`, and its stored attachment
  list keeps the real sizes. Calendar invitations and inline images up to
  64 KB stay, so invites work and most messages render. If the stripped copy
  doesn't parse to the same text and attachments, the full message is kept.
  The full message replaces the partial copy when:
  - an attachment is opened;
  - the raw message is requested;
  - a server draft is opened for editing;
  - a message is forwarded;
  - a message with larger inline images is opened.
  If the forwarded message's attachments couldn't be fetched, sending the
  forward fails rather than going out without them. With the AI folder set
  to include attachments or .eml files, copies are kept whole.
- **An optional storage limit per account** (`storage_limit_mb`). Before each
  batch, the account's oldest stored content is freed until it fits. A
  message downloads only if freeing older mail can make room for it, so the
  newest mail always wins and the same messages don't keep downloading and
  being freed. Freed messages stay listed. Their search entries and AI
  folder files stay as they were, and opening one downloads it again.

Schema 2 adds `messages.partial` and `messages.blob_size` (0031).

## Consequences

- Agents and search eventually see all mail's text. On a large mailbox this
  takes days of background syncing, and costs requests against Gmail and
  Graph quotas.
- Attachments mostly stay on the server, not just off the disk.
  `MailProvider::fetch_without_attachments` asks the server for the
  message's structure, downloads every part's headers and only the contents
  a stored copy keeps, and rebuilds the message (`satchel_mime::rebuild`) in
  the shape `strip_attachments` produces. Attachment sizes come from the
  structure. Background downloads, messages opened before they downloaded,
  and the IMAP recent window use it. `ensure_full` still downloads the whole
  message.
  - IMAP: `BODYSTRUCTURE`, then `BODY.PEEK[HEADER]`, each part's `.MIME`
    headers and the kept parts. Base64 sizes are estimated from the encoded
    size, which is all IMAP reports (within a few bytes for 76-character
    lines).
  - Gmail: `format=full`. Text comes inline; a kept inline image downloads
    by attachment id. Gmail lists headers as name/value pairs, so header
    folding and encoded words may come back decoded. The parse is the same.
  - JMAP: `bodyStructure` with text and HTML `bodyValues`. Other kept parts,
    like invitations and small inline images, download as blobs. Text comes
    decoded, so it's stored as UTF-8.
  - Graph only offers the whole MIME message (`$value`) or its own body and
    attachment objects, not a MIME structure. It still downloads in full and
    is stripped afterwards.
  Messages under 128 KB (`SLIM_FROM_BYTES`), or with nothing to leave out,
  download whole: asking for the structure first would cost an extra
  request (and Gmail quota) for nothing. Copies rebuilt this way are always
  `partial`, because their bytes aren't the server's, so the raw message
  still comes from the server. If a rebuild fails, the message downloads
  whole. With the AI folder copying attachments or .eml files, messages
  download whole as before.
- Space is counted per account, from what's stored on disk, not the space
  free on the drive. Copies shared between accounts count for each one.
- Content freed by the limit loses its body from search if the search index
  is rebuilt, or if a race re-indexes it as it's freed. An AI folder thread
  file that is rewritten, say because a reply arrived, has only the preview
  for freed messages.
- Supersedes the "fetched on demand" part of 0005. The content-addressed
  store itself is unchanged.
