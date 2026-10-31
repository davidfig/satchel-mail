# 0057. Calendar alerts scheduled with macOS

- **Status:** Accepted
- **Date:** 2026-10-30

## Context

Calendar alerts (0056) only went off while Satchel was running. Their
system notification was posted at that moment, had no buttons, and was
only shown while the window was in the background. A calendar's
alerts should work like the system Calendar's: they should show when the
app is quit, and Snooze and Dismiss should be right on the notification.

The notification plugin can't do either on macOS. It posts through the
deprecated `NSUserNotification` and has no scheduling or actions there.

## Decision

The Mac app uses Apple's User Notifications framework
(`UNUserNotificationCenter`, in `apps/desktop/src/mac_alerts.rs`). The
core still decides which alerts there are. The host only keeps the
system's schedule in step with it.

- **Ids known in advance.** An alert's id is
  `{event id}:{occurrence start, ms}:{minutes before}`, and its `due` is
  when it goes off (or when its snooze ends). `ListUpcomingAlerts` lists
  the ones coming up. `SnoozeAlert` and `DismissAlert` accept an alert
  that hasn't gone off in the core yet and record it, which covers a
  notification acted on while the app was quit.
- **Scheduling.** The host schedules the alerts for the next 7 days, at
  most 50 (macOS keeps 64 per app), as `alert/{id}@{due ms}`. It
  reschedules when calendars, alerts or settings change, and every 15
  minutes. Turning alerts off takes them all down.
- **Actions.** Each notification offers Snooze 5 minutes, Snooze 15
  minutes, Snooze 1 hour and Dismiss. Closing it dismisses the alert too.
  Clicking it opens the event: the app comes forward on that occurrence,
  and an app that was quit keeps the request until the UI asks for it.
- **One notification per alert.** While the window has focus, alerts
  show in the app's own panel and the system banner is held back.
  `AlertsChanged.closed` names alerts dismissed or snoozed anywhere, so
  their notifications are taken down. An alert the core fires that the
  system wasn't scheduled to show (it came up after the last resync) is
  posted then.
- **Permission** is asked the first time there's an alert to schedule. New-mail
  notifications go through the same framework, by way of the host's
  `notify` and `can_notify` commands. The UI no longer calls the
  notification plugin directly.

The framework only runs inside an app bundle. A bare binary (`tauri dev`),
Windows and Linux keep 0056's behaviour: the core fires alerts while the
app runs, and the UI posts a notification when the window isn't in focus.

## Consequences

- An alert scheduled with the system shows even if its event changed
  after the app quit; the next start reschedules.
- Alerts more than a week ahead only reach the system once the app has
  run within the week before them.
- Windows (scheduled toasts) and Linux (no scheduling in the
  notification spec) are separate work.
