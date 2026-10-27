# 0005. Content-addressed raw message storage

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

The same message can exist in several folders or be re-fetched; raw RFC 822
bytes are the ground truth for rendering, search and the AI mirror.

## Decision

Store raw messages as immutable files named by their BLAKE3 hash
(`blobs/ab/cdef….eml`), behind the `BlobStore` trait. Messages reference blobs
by hash; bodies older than the initial-sync window are fetched on demand
(superseded by 0036: all mail downloads in the background, without
attachment contents, within an optional storage limit).

## Consequences

- Duplicates cost nothing; writes are atomic (temp file + rename).
- The hosted version can swap in object storage (S3) behind the same trait.
