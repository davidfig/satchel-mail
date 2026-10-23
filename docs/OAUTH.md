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
2. Enable the **Gmail API** and **Google Calendar API**.
3. OAuth consent screen: External; add the scopes below; add yourself as a test user.
4. Credentials → Create credentials → OAuth client ID → **Desktop app**.
   Copy the client id and secret (desktop-app secrets are not confidential).

Scopes requested: `openid email profile https://mail.google.com/ https://www.googleapis.com/auth/calendar`.

`https://mail.google.com/` is a **restricted** scope. While the app is in
"Testing", up to 100 listed test users can sign in. Public distribution needs
Google's OAuth verification and — because the scope is restricted — an annual
third-party security assessment (CASA), especially for the hosted version.
Plan for this before launch.

## Microsoft Entra ID

1. <https://entra.microsoft.com/> → App registrations → New registration.
   Supported accounts: "Accounts in any organizational directory and personal
   Microsoft accounts" (multi-tenant + MSA).
2. Authentication → Add a platform → **Mobile and desktop applications** →
   custom redirect URI `http://localhost`. Enable "Allow public client flows".
3. API permissions (delegated): `Mail.ReadWrite`, `Mail.Send`,
   `Calendars.ReadWrite`, `User.Read`, `offline_access`, `openid`, `email`, `profile`.
4. Copy the Application (client) id.

Publisher verification is needed for a verified app name on the consent screen;
some organizations require admin consent for these permissions.
