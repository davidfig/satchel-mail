# 0071. The unread badge on Windows and Linux

- **Status:** Accepted
- **Date:** 2026-11-07

## Context
0070 put the unread inbox count on the Mac app's icon and left the other
platforms for later. Neither can use the Mac's call:

- Windows has no badge count. Its taskbar shows an overlay image in the corner
  of the app's button, which Tauri can set from RGBA pixels.
- On Linux, Tauri's `set_badge_count` loads libunity and only acts if the old
  Unity desktop is running, so most desktops would never see it. Docks that show
  counts, such as Ubuntu's and KDE's, listen for the
  `com.canonical.Unity.LauncherEntry` D-Bus signal instead.

## Decision
The host keeps the count on all three, sharing the event loop from 0070.

- **Windows:** draw the count ourselves, white digits on a red circle capped at
  "99+", from a small built-in pixel font, so no image or font dependency. Set
  the first count twice, because Windows ignores an overlay set before the
  taskbar button exists.
- **Linux:** send the signal directly with zbus, for the desktop file the
  bundler names `satchel.email.desktop`, and keep the connection open because
  docks drop the count when the sender goes away. zbus is already in Linux
  builds through notify-rust and keyring.

## Consequences
- The count shows on the taskbar or dock, but we don't expect it in Alt-Tab on
  either platform, unlike Cmd-Tab on the Mac.
- On Linux it's best effort. Docks that ignore the signal, such as plain GNOME
  without an extension, show nothing, and an AppImage only matches if something
  has installed its desktop file. There's no error to detect either case.
- On Windows the overlay is lost if Explorer restarts, until the count changes.
- Windows' overlay and the Linux signal can't run on a Mac, so changes to them
  need checking on those platforms by hand.
