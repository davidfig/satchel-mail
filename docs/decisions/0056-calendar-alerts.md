# 0056. Calendar alerts: VALARMs in the stored event, fired by the core

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

Satchel had no calendar alerts. The iCalendar parser skipped VALARM
blocks, the Google and Outlook converters dropped reminders, and nothing
watched the clock. Worse, saving an edit to a CalDAV event rebuilt it
from the editor's fields, so it deleted any alarm another client had set.

## Decision

**Storage.** Alerts live where everything else about an event lives: in its
stored iCalendar, as VALARMs. Each provider's converter writes them there.
Google's popup reminders become DISPLAY alarms and its email reminders
become EMAIL alarms. Alarms that come from a calendar's default reminders
are marked `X-SATCHEL-DEFAULT:TRUE`. Outlook's single reminder becomes one
DISPLAY alarm. No new event columns are needed.

**API.** `Event.alerts` is a list of `Alert { minutes_before }` built from
the DISPLAY and AUDIO alarms. A negative value means after the start. A
trigger relative to the end, or at a fixed time, is converted against the
master's times. On writes, `alerts: null` (or leaving it out) keeps the
stored alarms. Agents and older clients that don't know the field can't
wipe them.

**Edits keep what they don't change.** When an edit leaves the alerts as
they were, the stored VALARM blocks are carried over untouched, with every
property we don't model (ACKNOWLEDGED, X-WR-ALARMUID, sounds). When the
alerts change, new DISPLAY alarms replace the old DISPLAY and AUDIO ones.
EMAIL and other alarms are always kept. Writing back:
- CalDAV PUTs the iCalendar as is.
- Google gets `reminders: {useDefault: true}` when every alarm is still a
  default one. Otherwise it gets explicit overrides: popup and email, at
  most 5.
- Outlook gets `isReminderOn` and `reminderMinutesBeforeStart` from the
  first alert. Outlook holds only one reminder. The reminder is a personal
  setting, so for someone else's meeting it is patched on its own, without
  touching the rest of the event.

**Firing.** A core worker (Full mode only) expands the visible calendars'
occurrences from a day back to 4 weeks ahead. An alert fires once its time
has come, within the last hour, if the occurrence hasn't ended. A
fired alert is recorded in `calendar_alerts`, one row per event,
occurrence and alert, so it never fires twice, across restarts too. All-day
events take their alert times from local midnight on the host. The worker
skips cancelled events, events you declined, and hidden calendars. It
sleeps until the next alert is due, for at most a minute, and wakes early
when calendars or settings change.

**Acting on an alert.** `ListAlerts` returns the alerts that have fired and
are still open. `SnoozeAlert { alert_id, until }` brings one back later.
`DismissAlert { alert_id: null }` dismisses all of them. Every change sends
`CoreEvent::AlertsChanged { due }`, where `due` holds the alerts that just
fired, so the UI notifies only for those. In the UI:
- Open alerts stack in a panel with Snooze (5, 10, 15 minutes, 1 hour, or
  until the start), Open and Dismiss.
- A system notification goes out when the window doesn't have focus.
- The event editor and the details view set an event's alerts.
- General settings turn alerts off and choose the default alert for new
  events.

Alert commands are user-only. Agents can see `Event.alerts` and propose
changes to it, like any other event field.

## Consequences

- Alerts work only while a Full-mode host (the app or `satchel serve`)
  runs; the Mac app also schedules them with macOS (0057). Phones and
  other clients keep their own alerts from the same synced alarms.
- Dismissing here doesn't tell other devices. Writing ACKNOWLEDGED back
  would cost a server write per alert.
- Alerts that come due while the app is closed show if the app opens
  within an hour of them and the event hasn't ended. Older ones are
  dropped rather than flooding the screen.
- Outlook keeps only the first of several alerts.
