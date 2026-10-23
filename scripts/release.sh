#!/usr/bin/env bash
# Build release artifacts for the current (or $TARGET) platform:
#   - the `satchel` CLI (also bundled into the app as a sidecar for MCP)
#   - the desktop app bundle(s) via Tauri
#
# Optional environment:
#   TARGET                     Rust target triple (default: host)
#   BUNDLES                    e.g. "app" or "app,dmg" (default: Tauri config)
#   SATCHEL_UPDATE_URL         update feed, e.g. https://example.com/satchel/{{target}}/{{arch}}/{{current_version}}
#   SATCHEL_UPDATE_PUBKEY      contents of the updater public key (tauri signer generate)
#   TAURI_SIGNING_PRIVATE_KEY  signs update artifacts (required with SATCHEL_UPDATE_URL)
#   SATCHEL_GOOGLE_CLIENT_ID / SATCHEL_GOOGLE_CLIENT_SECRET / SATCHEL_MS_CLIENT_ID
#                              baked in so Google / Microsoft sign-in works (docs/OAUTH.md)
#   APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID
#                              macOS signing + notarization (handled by Tauri)

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TRIPLE="${TARGET:-$(rustc -vV | sed -n 's/host: //p')}"
EXE=""
[[ "$TRIPLE" == *windows* ]] && EXE=".exe"

echo "==> UI"
(cd ui && npm ci --silent && npm run build --silent)

echo "==> satchel CLI ($TRIPLE)"
cargo build --release -p satchel-cli --target "$TRIPLE"
mkdir -p apps/desktop/binaries
cp "target/$TRIPLE/release/satchel$EXE" "apps/desktop/binaries/satchel-$TRIPLE$EXE"

ARGS=(build --target "$TRIPLE")
[[ -n "${BUNDLES:-}" ]] && ARGS+=(--bundles "$BUNDLES")

if [[ -n "${SATCHEL_UPDATE_URL:-}" ]]; then
  : "${SATCHEL_UPDATE_PUBKEY:?SATCHEL_UPDATE_PUBKEY is required with SATCHEL_UPDATE_URL}"
  : "${TAURI_SIGNING_PRIVATE_KEY:?TAURI_SIGNING_PRIVATE_KEY is required with SATCHEL_UPDATE_URL}"
  cat > apps/desktop/tauri.updater.json <<EOF
{
  "bundle": { "createUpdaterArtifacts": true },
  "plugins": { "updater": { "endpoints": ["$SATCHEL_UPDATE_URL"], "pubkey": "$SATCHEL_UPDATE_PUBKEY" } }
}
EOF
  ARGS+=(--features updater --config tauri.updater.json)
fi

echo "==> desktop app"
(cd apps/desktop && "$ROOT/ui/node_modules/.bin/tauri" "${ARGS[@]}")

echo
echo "Artifacts:"
find "target/$TRIPLE/release/bundle" -maxdepth 2 -mindepth 2 2>/dev/null | sed 's/^/  /'
echo "  target/$TRIPLE/release/satchel$EXE"
