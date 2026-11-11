# 0079. An idle AI folder pass writes only what changed

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

Every 20 seconds the AI folder writer rebuilt its indexes, history,
agenda, calendar files and contact files, and compared each with what was
on disk (`write_if_changed`). On a real mailbox (88k messages, 3.5k events,
1.3k contacts) a pass where nothing had changed took about 0.55 s:
contacts 176 ms, calendar 168 ms, indexes 143 ms, history 50 ms. That's
steady work on an idle machine, around the clock.

## Decision

- **Each part has a stamp** of what it's built from (`Store::stamps`): row
  counts and version sums of messages, threads, mailboxes, written thread
  folders and favorite senders for the mail indexes and history; of events
  and calendars for the calendar; of contacts, address books and cards (plus
  the mail stamp, for message counts) for contacts. Every edit bumps a row's
  version, so equal stamps mean nothing that part shows has changed. The
  stamp is combined with the folder's settings and the day (the hour, for
  the agenda's moving dates).
- **A part is written only when its stamp changed** since it was last
  written, or when a thread folder was written in the same pass (the
  indexes link to them), and **every part every ten minutes** regardless.
- `Engine::refresh_mirror` forces the next pass to write everything, as
  does `recheck_mirror` (which also checks every thread folder at once
  instead of waiting for the daily check).

## Consequences

- An idle pass takes about 36 ms, most of it summing message versions.
- A file changed or deleted behind Satchel's back (an index, the agenda) is
  put back within ten minutes, not 20 seconds. A missing thread folder is
  still found by the daily check, or sooner if the indexes notice it.
- A change that doesn't bump a version or a count (a raw SQL edit by hand)
  waits for the ten-minute write.
