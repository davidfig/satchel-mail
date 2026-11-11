# 0074. One stored copy of each message, compressed; everything else is derived

- **Status:** Accepted
- **Date:** 2026-11-10
- **Amends:** 0005 (blobs are compressed), 0036 (sizes count compressed bytes)

## Context

It looked as if every message was stored twice: once in the data directory and
once in the AI folder. On a real data directory (four accounts, 88k messages):

| What | Size | What it is |
|---|---|---|
| `blobs/` | 3.7 GB | every message without attachment contents (0036) |
| `satchel.db` | 400 MB | metadata, threads, and a 175 MB search index |
| `~/Satchel/` | 109 MB | one year of `thread.md` text, quotes removed |

So the AI folder is not a second copy of the mail. It's about 3% of the
store. The store itself is the cost: 2.8 GB of it is ordinary messages under
128 KB, mostly HTML newsletters. A sample of 400 of them gzipped to a quarter
of their size.

We considered keeping only the AI folder's text, but it can't stand in for the
original. It loses the HTML, the headers that replies and threading need, the
DKIM signatures that sender rules check (0034, 0061), inline images, and
invitations. And it's a folder any local process may write to (0022), so it
can't be the source of truth.

## Decision

- **Each message is stored once, gzip-compressed** (`blobs/ab/cdef….gz`).
  The hash is still BLAKE3 of the *uncompressed* bytes, so ids, dedup and
  every reference in the database are unchanged. `BlobStore::put` returns the
  hash and the bytes on disk. `get` returns the original bytes.
- **Older copies are converted in the background.** `.eml` files written
  before this are still read. A worker compresses them in batches of 200,
  checks each against its hash first (a mismatch is left alone and logged),
  and records each new size. When none are left it sets `blob_format` to
  `gz-v1` and stops for good. Storing a message over an uncompressed copy
  also replaces it.
- **`blob_size` counts compressed bytes**, so the storage limit (0036) still
  means space on disk, and the same limit now holds about four times as much
  mail.
- **Everything else is derived from that one copy and never read back as
  truth.** The search index is contentless (its text lives in the blobs).
  The AI folder is a view written for agents. It can be deleted at any time
  and is rebuilt from the store.
- Compression uses `flate2` (gzip, level 6, the pure-Rust `miniz_oxide`
  back end). It's already in the tree through `mail-auth` and `png`, at the
  same version.

## Consequences

- About 3.7 GB becomes about 0.9 GB on the data directory above. Converting
  it took about 50 seconds of CPU in the estimate, once.
- Reading a stored message adds about 0.04 ms of decompression (around 1 GB/s
  for an average 43 KB message). Storing one adds about 0.6 ms. Both are
  small next to parsing, rendering and the network.
- Stored copies are no longer `.eml` files you can open from disk. The raw
  message is still available from the app ("raw message", forwards), and the
  AI folder's optional `.eml` files are written uncompressed.
- The hosted version's object store (0018) can keep the same compressed bytes
  behind the same trait.
- Temporary files get unique names, so two writers of the same message can't
  interleave in one file.
- What the AI folder should hold (recent text only, attachments, or not exist
  at all) is a separate question, to be decided with evidence from agents
  using it.
