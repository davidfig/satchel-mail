# 0049. Events move between calendars, unless they have guests

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
The event editor locked the calendar of an existing event, and `update_event`
refused a new `calendar_id`. People do want to move an event, e.g. a dentist
appointment put on the work calendar by mistake, even to another account.

No provider we use has a move that works across accounts, so a move has to
be a new copy in the target calendar plus a delete of the old one. For an
event with guests, servers read that as the organizer cancelling: Graph and
CalDAV scheduling send a cancellation for the delete, and the new copy sends
fresh invitations. Only Google can suppress both (`sendUpdates=none`). If
we're a guest instead, deleting our copy can tell the organizer we declined.

## Decision
`update_event` with a different calendar moves the event: same UID, cancelled
dates and changed occurrences kept, sequence bumped. It's written to the new
calendar (`cal_put`) and deleted from the old one with `notify: false`. Both
calendars must be writable.

Events that have or had guests can't move; the core says why, and the editor
disables the calendar picker with a one-line explanation. Dropping the guests
in the same edit doesn't get around it.

Agent proposals still can't change an event's calendar: an agent naming
another calendar must not be able to probe it, and moves are rare enough to
leave to the user.

## Consequences
- Moving gives the event a new local id.
- The put and delete run on separate account queues, so for a moment both
  servers may have the event, or (if the put fails and is retrying) neither
  does. The local copy is in the new calendar throughout.
- Moving events with guests could come later for Google only, or as an
  explicit "move and re-invite" the user confirms.
