# Releasing

Releases are built by `.github/workflows/release.yml` for Mac (Apple silicon
and Intel), Windows and Linux, signed, and attached to a **draft** GitHub
release. Why it's signed this way: [0059](decisions/0059-signing-releases.md).

## Making a release

1. Bump `version` in `apps/desktop/tauri.conf.json` (and the workspace
   `Cargo.toml`).
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. When the workflow finishes, open the draft under Releases, check the notes
   and files, and publish it. The site's download buttons point at
   `releases/latest/download/<file>`, so they follow the newest published
   release.
4. **First release only:** the site hides its downloads until there is
   something behind them. In `site/index.html`, remove `data-unreleased` from
   `<section id="get">` and delete the "Not released yet" notice, and take the
   "Status: not released yet" section out of the README. Trim the "Early
   release" notice as Google and Microsoft finish their reviews.

A tag fails unless the Mac and Windows signing keys below are set. To test the
workflow without releasing, push a change to the workflow file on a branch, or
run it from the Actions tab.

## One-time setup

Secrets and variables go in GitHub: Settings → Secrets and variables →
Actions.

### Mac: Developer ID and notarization

1. Join the [Apple Developer Program](https://developer.apple.com/programs/)
   ($99 a year). The free personal team can't sign apps for other people.
2. In Xcode → Settings → Accounts → Manage Certificates, add a
   **Developer ID Application** certificate. In Keychain Access, export it
   with its private key as a `.p12` with a password.
3. Add secrets:
   - `APPLE_CERTIFICATE`: `base64 -i DeveloperID.p12 | pbcopy`, then paste
   - `APPLE_CERTIFICATE_PASSWORD`: the export password
   - `APPLE_SIGNING_IDENTITY`: the name `security find-identity -v -p codesigning`
     shows, e.g. `Developer ID Application: Jane Doe (AB12CD34EF)`
4. For notarization, create an App Store Connect API key (App Store Connect →
   Users and Access → Integrations → Team Keys, role Developer) and add:
   - `APPLE_API_ISSUER`: the issuer ID
   - `APPLE_API_KEY`: the key ID
   - `APPLE_API_PRIVATE_KEY`: the contents of the downloaded `.p8`

   Or, instead of a key: `APPLE_ID`, `APPLE_PASSWORD` (an
   [app-specific password](https://account.apple.com)) and `APPLE_TEAM_ID`.

### Windows: Azure Artifact Signing

1. In the Azure portal, create an **Artifact Signing** account (formerly
   Trusted Signing), then an identity validation (public trust) and a
   **certificate profile**. Validation can take a few days.
2. Create an app registration (Microsoft Entra ID → App registrations), give
   it a client secret, and on the signing account grant it the
   **Artifact Signing Certificate Profile Signer** role.
3. Add secrets `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, and
   variables:
   - `AZURE_ARTIFACT_SIGNING_ENDPOINT`: your region's endpoint, e.g.
     `https://eus.codesigning.azure.net`
   - `AZURE_ARTIFACT_SIGNING_ACCOUNT`: the account name
   - `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE`: the profile name

If Artifact Signing isn't available to you, an OV certificate in a cloud HSM
(SSL.com eSigner, DigiCert KeyLocker) works too: change the `signCommand` the
workflow writes to that provider's signing tool.

### Linux

Nothing to set up. Linux builds aren't code-signed; each release has a
`SHA256SUMS` file and, once the repository is public, GitHub build
attestations. Anyone can check a download:

```bash
gh attestation verify satchel.email-linux-x64.AppImage --repo davidfig/satchel-mail
```

### Sign-in

Google and Microsoft sign-in need `SATCHEL_GOOGLE_CLIENT_ID`,
`SATCHEL_GOOGLE_CLIENT_SECRET` and `SATCHEL_MS_CLIENT_ID` (see
[OAUTH.md](OAUTH.md)). Without them a build works, but those accounts can't be
added.

## Checking a build

Each build job ends with **Check signatures**: `codesign`, `stapler` and
`spctl` on the Mac app and CLI, and `Get-AuthenticodeSignature` on the Windows
installer and CLI. A build without keys says it's unsigned and moves on.

## The website

`site/` is plain HTML with no build. Cloudflare Pages serves it straight from
this repository: Workers & Pages → Create → **Pages** → Connect to Git → this
repo, with framework preset None, no build command, build output directory
`site`, and production branch `main`. Every push to `main` deploys, and other
branches get preview URLs. Attach `satchel.email` and `www.satchel.email` as
custom domains on the project; the zone is in the same Cloudflare account, so
Cloudflare adds the web records and certificate itself.

Cloudflare reads two files from the folder: `_headers` (the security headers,
including the hash of the inline theme script in every page; its comment says
how to recompute it) and `404.html`.
