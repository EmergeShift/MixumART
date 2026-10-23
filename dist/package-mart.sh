#!/bin/bash
# Build MixumART with the mandatory mixum CLI/host/extension bundled.
# The CLI is a hard dependency: Mart users must not install it separately.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
DIST="$ROOT/dist"
RES="$ROOT/apps/desktop/src-tauri/resources"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

bash "$ROOT/dist/package-cli.sh" "$TMP"
TARBALL="$(ls "$TMP"/mixum-cli-*.tar.gz | head -1)"
tar -xzf "$TARBALL" -C "$TMP"
CLI_DIR="$(find "$TMP" -maxdepth 1 -type d -name 'mixum-cli-*' | head -1)"
[ -n "$CLI_DIR" ] || { echo 'CLI bundle missing' >&2; exit 1; }

rm -rf "$RES"
mkdir -p "$RES/bin" "$RES/extensions"
cp "$CLI_DIR/bin/mixum" "$RES/bin/mixum"
cp "$CLI_DIR/bin/mixum-browser-host" "$RES/bin/mixum-browser-host"
cp -a "$CLI_DIR/share/mixum/extensions/mixum-browser" "$RES/extensions/mixum-browser"
rm -rf "$RES/extensions/mixum-browser/store"
cp "$ROOT/docs/browser-early-access.md" "$RES/BROWSER-README.md"

cd "$ROOT/apps/desktop"
npx tauri build "$@"

# Tauri bundles the app before arbitrary resource copy; inject the mandatory
# CLI/host/extension, then re-sign ad-hoc and rebuild the DMG.
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
APP="$(find "$TARGET_DIR" -path '*/bundle/macos/*.app' -type d -print -quit)"
[ -n "$APP" ] || { echo 'Tauri app bundle not found' >&2; exit 1; }
mkdir -p "$APP/Contents/Resources/bin" "$APP/Contents/Resources/extensions"
rm -rf "$APP/Contents/Resources/bin" "$APP/Contents/Resources/extensions"
cp -a "$RES/bin/." "$APP/Contents/Resources/bin/"
cp -a "$RES/extensions/." "$APP/Contents/Resources/extensions/"
cp "$RES/BROWSER-README.md" "$APP/Contents/Resources/BROWSER-README.md"
codesign --force --deep --sign - "$APP" 2>/dev/null || true
if command -v hdiutil >/dev/null 2>&1; then
  DMG_DIR="$(cd "$(dirname "$APP")/../dmg" && pwd)"
  mkdir -p "$DMG_DIR"
  find "$DMG_DIR" -maxdepth 1 -name '*.dmg' -delete
  DMG="${DMG_DIR}/$(basename "$APP" .app).dmg"
  hdiutil create -volname "$(basename "$APP" .app)" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
fi
echo "bundled CLI in $APP/Contents/Resources/bin"
