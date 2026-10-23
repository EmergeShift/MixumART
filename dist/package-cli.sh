#!/bin/bash
# Build a portable mixum CLI bundle:
#   mixum, mixum-browser-host, browser extension sources, README.
# Usage: dist/package-cli.sh [output-dir]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/dist}"
VERSION="$(python3 - "$ROOT/Cargo.toml" <<'PY'
import sys,re
text=open(sys.argv[1]).read()
m=re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
print(m.group(1) if m else '0.0.0')
PY
)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
NAME="mixum-cli-${VERSION}-${OS}-${ARCH}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "==> build mixum CLI + host"
cargo build --release -p mixum-cli --bins

mkdir -p "$STAGE/$NAME/bin" "$STAGE/$NAME/share/mixum/extensions"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cp "$TARGET_DIR/release/mixum" "$STAGE/$NAME/bin/mixum"
cp "$TARGET_DIR/release/mixum-browser-host" "$STAGE/$NAME/bin/mixum-browser-host"
cp -a "$ROOT/extensions/mixum-browser" "$STAGE/$NAME/share/mixum/extensions/mixum-browser"
rm -f "$STAGE/$NAME/share/mixum/extensions/mixum-browser/"{*.tpl,*.zip}
rm -rf "$STAGE/$NAME/share/mixum/extensions/mixum-browser/store"
cp "$ROOT/docs/browser-early-access.md" "$STAGE/$NAME/README.md"

mkdir -p "$OUT"
tar -C "$STAGE" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "wrote $OUT/$NAME.tar.gz"
