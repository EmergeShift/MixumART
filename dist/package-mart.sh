#!/bin/bash
# Build MixumART with the mandatory mixum CLI/host/extension bundled.
# The CLI is a hard dependency: Mart users must not install it separately.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RES="$ROOT/apps/desktop/src-tauri/resources"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Resolve a relative CARGO_TARGET_DIR once so cargo and the post-build copy
# below agree on where the bundle lands (tauri build changes cwd).
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ "${CARGO_TARGET_DIR#/}" = "$CARGO_TARGET_DIR" ]; then
  CARGO_TARGET_DIR="$ROOT/$CARGO_TARGET_DIR"
  export CARGO_TARGET_DIR
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

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
APP="$(find "$TARGET_DIR" -type d -path '*/release/bundle/macos/mixum Agent Runtime.app' 2>/dev/null | head -1)"
[ -n "$APP" ] || { echo 'app bundle missing' >&2; exit 1; }
rm -rf "$APP/Contents/Resources/bin" "$APP/Contents/Resources/extensions"
cp -a "$RES/bin" "$APP/Contents/Resources/bin"
cp -a "$RES/extensions" "$APP/Contents/Resources/extensions"
cp "$RES/BROWSER-README.md" "$APP/Contents/Resources/BROWSER-README.md"
codesign --force --deep --sign - "$APP" 2>/dev/null || true

VERSION="$(python3 - "$ROOT/apps/desktop/src-tauri/tauri.conf.json" <<'PY'
import json, sys
print(json.load(open(sys.argv[1])).get("version", "0.0.0"))
PY
)"
DMG_DIR="$(dirname "$(dirname "$APP")")/dmg"
DMG="$(find "$DMG_DIR" -maxdepth 1 -type f -name "mixum Agent Runtime_${VERSION}_*.dmg" 2>/dev/null | head -1)"
[ -n "$DMG" ] || DMG="$DMG_DIR/mixum Agent Runtime_${VERSION}_$(uname -m).dmg"
if command -v hdiutil >/dev/null 2>&1; then
  mkdir -p "$DMG_DIR"
  rm -f "$DMG"
  hdiutil create -volname "mixum Agent Runtime" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
fi
echo "bundled CLI in $APP/Contents/Resources/bin"
