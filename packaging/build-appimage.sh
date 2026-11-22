#!/usr/bin/env bash
# Build dist/DonateHub-x86_64.AppImage with the Tauri bundler.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! cargo tauri --version >/dev/null 2>&1; then
    cargo install tauri-cli --locked
fi

# Updater artifacts need the signing key: take the local one if present, otherwise build without them.
KEY="$HOME/.tauri/donatehub.key"
EXTRA=()
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -f "$KEY" ]; then
    TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$KEY.password" 2>/dev/null || true)"
    export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    EXTRA=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
fi

# NO_STRIP: linuxdeploy's bundled strip is too old for current system libraries; the binary is stripped by cargo.
NO_STRIP=1 cargo tauri build --bundles appimage --config src-tauri/tauri.conf.json "${EXTRA[@]}"

mkdir -p dist
cp -f "$(ls -t target/release/bundle/appimage/*.AppImage | head -1)" dist/DonateHub-x86_64.AppImage
ls -lh dist/DonateHub-x86_64.AppImage
