# 0011. Microsoft Graph calendars synced as instances

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Graph exposes series masters without a simple delta, but `calendarView/delta`
returns expanded occurrences with change tracking.

## Decision

Sync `calendarView/delta` over −180/+540 days and store each occurrence as a
standalone event keyed by its Graph id. Organizers edit with PATCH; attendees
respond through the accept/decline/tentativelyAccept endpoints. New recurring
events convert a supported RRULE subset to Graph's recurrence pattern.

## Consequences

- Editing a series from an occurrence edits only that occurrence.
- Events outside the window aren't shown.
