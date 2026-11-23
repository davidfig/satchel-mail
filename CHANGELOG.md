# Changelog

What's new in each release of satchel.email. The newest is first.

<!--
How to add a release: add a section headed `## X.Y.Z (YYYY-MM-DD)` above the
others before you tag it. Write each line for the people who use the app (what
they'll notice), not as a commit list. Then run
`python3 scripts/changelog.py html` to update site/changelog.html. The release
workflow uses the section as the GitHub release notes and fails if it's missing
or the page is out of date. See docs/RELEASING.md.
-->

## 0.1.20 (2026-11-23)

- Some newsletters no longer slide down the message window as you read them; they open at the top at their real height.
- A newsletter's title no longer shows up as a stray line above it.

## 0.1.19 (2026-11-15)

- Unsubscribing also moves the mail to Trash. To keep it instead, turn off "Move mail to Trash when you unsubscribe from it" in Settings → General.
- Help → What's New opens inside the app, and works offline. When an update is ready to install, its What's new button shows what's in it before you restart.

## 0.1.18 (2026-11-15)

- Help → What's New opens a real changelog at satchel.email/changelog, instead of an almost empty GitHub page.
- The "ready to install" toast has a What's new button, and after an update the app says which version you're on, with a link to what changed.

## 0.1.17 (2026-11-15)

- The app checks for updates every hour instead of every six hours, so fixes reach you sooner.
- The update-check toast fits its text.

## 0.1.16 (2026-11-14)

- Fewer reply warnings: a different Reply-To address is flagged only on mail from people you've written to, where a reply could be hijacked, not on everyday newsletters and notifications.
- The update-check toast keeps the same width while an update downloads.

## 0.1.15 (2026-11-14)

- Alerts have a Dismiss button.
- The app says when you're offline.

## 0.1.14 (2026-11-11)

- Settings and AI proposals can sync between your computers through a shared folder, such as iCloud Drive or Dropbox.
- Each message is stored once, compressed, so the app's data takes less space.
- The AI folder (`~/Satchel`) is smaller, fills in your older mail over time, and costs less to keep up to date. Threads missing from it are repaired.

## 0.1.13 (2026-11-10)

- Right-clicking a conversation in the Mac app opens satchel.email's menu again, instead of the system's.

## 0.1.12 (2026-11-08)

- All-day events show on their own days in the calendar.
- No scam warning for people you've written to.
- When a folder opens, the cursor is on its first conversation (or where you left it), so j/k and Enter work right away.

## 0.1.11 (2026-11-08)

- Check for Updates says what it's doing.

## 0.1.10 (2026-11-08)

- Add send-as addresses to an account and choose which one to send from.
- Replies proposed by AI agents go out from the send-as address the conversation reached, like yours do.

## 0.1.9 (2026-11-07)

- The "Sending" toast counts down, and has a Send now button.

## 0.1.8 (2026-11-07)

- The unread count shows on the app icon on Windows and Linux too.

## 0.1.7 (2026-11-07)

- The Mac app's Dock icon shows how many inbox messages are unread.

## 0.1.6 (2026-11-06)

- A fuller Help menu: Send Feedback, What's New, Check for Updates, and links to the website, privacy policy and terms.

## 0.1.5 (2026-11-06)

- A signed-out account is hard to miss: a banner with a Sign in button, and a notification if it stays signed out. Password accounts can enter a new password without being removed.
- Gmail mail shows up sooner, and the app checks for mail when you come back to the window.
- A link that goes somewhere other than it shows now asks when you click it. Mail from verified senders no longer gets a scam banner for this.

## 0.1.4 (2026-11-05)

- Smart folders: save a search and it appears under Mail.
- Deleted mail stays deleted, and the next conversation opens.
- A Google sign-in that doesn't allow Gmail access is refused with an explanation, instead of half working.

## 0.1.3 (2026-11-04)

- A conversation shows which of your addresses it reached.
- Moving the open conversation away opens the next one.
- Message previews show real text, not HTML codes like `&amp;`.

## 0.1.2 (2026-11-04)

- Accounts whose sign-in was revoked can sign in again.
- The Mac download itself is notarized by Apple, not only the app inside it.

## 0.1.1 (2026-11-03)

The first public release.

- HTML messages show right away, instead of waiting for every remote image to load.
- Sync no longer undoes changes that are still on their way to the server.

## 0.1.0 (2026-11-03)

A test build, not released to the public. What the first version can do:

- Mail, calendar and contacts for Gmail, Microsoft, and any IMAP/SMTP, JMAP, CalDAV or CardDAV account, kept on your computer.
- AI agents read your mail through the `~/Satchel` folder or MCP and can only propose changes, which you review.
- Attachments open in the app, and links that try to deceive are flagged.
- Calendar alerts, a reworked event editor with place suggestions, and favorite senders.
- Mail that expects a reply is told apart from lists and automated mail.
- Select conversations with Shift+click, Shift+Arrow or Cmd+A, and copy a conversation for AI.
- Installed apps update themselves from new releases.
