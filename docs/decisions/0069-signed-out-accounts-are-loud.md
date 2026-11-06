# 0069. A signed-out account is loud until it's fixed

- **Status:** Accepted
- **Date:** 2026-11-06

## Context
When a provider signs Satchel out of an account, the only signs were a small
red dot on the account's sidebar header and a footer status linking to
settings. Nothing looks broken: the mail just stops, and a combined inbox keeps
filling from the other accounts. The sign-in button was only in settings.

A sync failure also counted as signed out if an untyped error's text mentioned
"authentication" or a failed login, which is harmless behind a dot but not
behind a banner.

## Decision
- A banner above every view, one per signed-out account, with a button that
  starts the sign-in (or asks for a new password for a password account). It
  can't be dismissed and leaves when the account signs back in or is removed.
- In the sidebar, a labeled "Sign in" button replaces the dot, the last-synced
  time and the new-folder button on that account's row. The footer starts the
  sign-in directly when that's the only problem.
- One system notification per sign-out, if the account is still signed out after
  60 seconds and the app isn't focused. It isn't gated on the new-mail
  notification setting, which is about mail, and never prompts for permission.
- Only a typed `ErrorCode::Auth` from a provider marks an account signed out.
  Offline and other sync errors stay in the footer: they fix themselves or need
  a different remedy.

## Consequences
- A banner takes vertical space until it's dealt with; that's the point, and
  removing the account is the way out for an account the user has abandoned.
- The notification repeats once per app launch while an account stays signed
  out, since what's been notified isn't saved.
- An untyped error that really is a sign-in failure now shows as a sync error.
  Providers should keep raising typed auth errors.
- Password accounts can now update their password, from the banner, the sidebar
  or settings, instead of removing and re-adding the account. The server checks
  it before it's saved, so a mistyped one leaves the old one in place.
