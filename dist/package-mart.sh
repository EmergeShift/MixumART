#!/bin/bash
# Build MixumART with the mandatory mixum CLI/host/extension bundled.
# The CLI is a hard dependency: Mart users must not install it separately.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RES="$ROOT/apps/desktop/src-tauri/resources"
TMP="$(mktemp -d)"
CONF="$ROOT/apps/desktop/src-tauri/tauri.release.conf.json"
trap 'rm -rf "$TMP"; rm -f "$CONF"' EXIT

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

cat > "$CONF" <<'JSON'
{
  "bundle": {
    "resources": {
      "resources/bin/mixum": "bin/mixum",
      "resources/bin/mixum-browser-host": "bin/mixum-browser-host",
      "resources/extensions/mixum-browser": "extensions/mixum-browser",
      "resources/BROWSER-README.md": "BROWSER-README.md"
    }
  }
}
JSON

cd "$ROOT/apps/desktop"
npx tauri build --config "$CONF" "$@"

echo "bundled CLI resources were staged in $RES"

