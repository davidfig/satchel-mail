# Google and Microsoft sign-in

Gmail and Microsoft 365 accounts use OAuth 2.0 with PKCE and a loopback
redirect (`http://127.0.0.1:<random port>/callback`, RFC 8252). Satchel needs a
client id for each provider, supplied at runtime or baked into release builds:

| Variable | Provider |
|---|---|
| `SATCHEL_GOOGLE_CLIENT_ID`, `SATCHEL_GOOGLE_CLIENT_SECRET` | Google |
| `SATCHEL_MS_CLIENT_ID` (and optionally `SATCHEL_MS_TENANT`, default `common`) | Microsoft |

Set them in the environment when running, or when building (`scripts/release.sh`
compiles them in with `option_env!`). Without them the account presets show
"Sign-in for this provider isn't configured".

## Google Cloud

1. Create a project at <https://console.cloud.google.com/>.
2. Enable the **Gmail API**, **Google Calendar API** and **People API** (contacts).
3. OAuth consent screen: External; add the scopes below; add yourself as a test user.
4. Credentials → Create credentials → OAuth client ID → **Desktop app**.
   Copy the client id and secret (desktop-app secrets are not confidential).

Scopes requested: `openid email profile https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/contacts`.

The contacts scope was added later (decision 0026). Accounts signed in before
that keep working; their contacts don't sync, and Settings → Accounts offers
"Reconnect to sync contacts", which signs in again and updates the account.

`https://mail.google.com/` is a **restricted** scope. While the app is in
"Testing", up to 100 listed test users can sign in, and their sign-ins expire
after 7 days. Public distribution needs Google's OAuth verification (0060).

### Getting verified

1. **Domain:** verify `satchel.email` in Search Console with an account that is
   an Owner or Editor of the Cloud project.
2. **Branding:** name, the logo in `docs/brand/google-consent-logo-120.png`,
   support email, `https://satchel.email`, `https://satchel.email/privacy`,
   `https://satchel.email/terms`, and the authorized domain. Verify and publish
   the branding.
3. **Audience:** publish to production. Until Google approves, people outside
   the test list see the unverified-app warning and count toward the 100-user
   cap.
4. **Data access:** declare the scopes above and say why narrower ones won't do.
   The full mail scope is there only for permanent delete
   (`messages.batchDelete`: Delete forever and Empty Trash or Spam); Google's
   [scope table](https://developers.google.com/workspace/gmail/api/auth/scopes)
   names that as its use, and `gmail.modify` can't do it. The permitted
   application type is an email client.
5. **Demo video:** unlisted on YouTube, in English. Show the consent screen with
   the client id in the address bar, then each scope in use.
6. **Security assessment (CASA):** Google's Trust and Safety team says when it's
   time. Don't book a lab first. The Gmail
   [scope table](https://developers.google.com/workspace/gmail/api/auth/scopes)
   and [verification guide](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification)
   tie the assessment to data on or through a third-party server, and this app
   has none, while the [help page](https://support.google.com/cloud/answer/13465431)
   says it applies to every restricted scope. So ask in the submission. A hosted
   version would need it regardless.

Google's consent screen lets people untick scopes. Sign-in refuses an account
without the mail scope; without calendar or contacts, those just don't sync.

## Microsoft Entra ID

1. <https://entra.microsoft.com/> → App registrations → New registration.
   Supported accounts: "Accounts in any organizational directory and personal
   Microsoft accounts" (multi-tenant + MSA).
2. Authentication → Add a platform → **Mobile and desktop applications**, and
   enable "Allow public client flows". Satchel signs in through
   `http://127.0.0.1:<random port>/callback`. Entra ignores the port of a
   loopback redirect but matches the host and the path, so register exactly
   `http://127.0.0.1/callback` (not `http://localhost`). The portal's text box
   refuses `http` loopback addresses, so add it in **Manifest** instead, under
   `replyUrlsWithType`:
   `{ "url": "http://127.0.0.1/callback", "type": "InstalledClient" }`.
   (See Microsoft's
   [redirect URI rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url#localhost-exceptions).)
3. API permissions (delegated): `Mail.ReadWrite`, `Mail.Send`,
   `Calendars.ReadWrite`, `Contacts.ReadWrite`, `User.Read`, `offline_access`,
   `openid`, `email`, `profile`. (`Contacts.ReadWrite` came later; older sign-ins
   are asked to reconnect, as for Google.)
4. Copy the Application (client) id.

### Publisher verification

Until a publisher is verified, the consent screen says "unverified", and
organizations that allow consent only for verified publishers need an admin to
approve the app. [Verification](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview)
is free but needs:

- A verified [Microsoft AI Cloud Partner Program](https://partner.microsoft.com/membership)
  account, at the partner global account level. Its checks are for a registered
  business, so a person needs at least a registered sole proprietorship, and
  whether that's enough is unconfirmed.
- That account's email domain matching the app's publisher domain, or a
  DNS-verified custom domain on the tenant. `*.onmicrosoft.com` isn't accepted.
- An app registered with a work or school account, not a personal Microsoft
  account. If the registration has to be redone, the client id changes, so
  ship a release with the new id and expect people to sign in again.
- The tenant linked to the partner account, and a person with an application
  admin role in Entra and an admin role in Partner Center, signed in with MFA.

`site/.well-known/microsoft-identity-association.json` proves the app's link to
`satchel.email` for the publisher domain step. To set it: App registrations →
the app → Branding & properties → Publisher domain.
