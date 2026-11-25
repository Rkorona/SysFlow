#!/bin/sh
# Pack the SysFlow module zip.
#   tools/pack-module.sh [path-to-arm64-binary]
# Default binary: core/target/aarch64-linux-android/release/sysflow
# Output: dist/sysflow-module-v<version>.zip (+ .sha256)
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/core/target/aarch64-linux-android/release/sysflow}"
[ -f "$BIN" ] || { echo "binary not found: $BIN" >&2; exit 1; }

VERSION="$(sed -n 's/^version=//p' "$ROOT/module/module.prop" | head -n 1)"
[ -n "$VERSION" ] || { echo "no version in module.prop" >&2; exit 1; }

[ -f "$ROOT/module/webroot/index.html" ] || {
  echo "module/webroot/index.html is missing; the WebUI is a static asset under module/webroot" >&2
  exit 1
}

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp "$ROOT/module/module.prop" "$ROOT/module/customize.sh" "$ROOT/module/service.sh" \
   "$ROOT/module/uninstall.sh" \
   "$ROOT/module/migrate-legacy.sh" "$ROOT/module/migrate-legacy-doze.sh" "$STAGE/"
cp -r "$ROOT/module/webroot" "$STAGE/webroot"
mkdir -p "$STAGE/bin/arm64-v8a"
cp "$BIN" "$STAGE/bin/arm64-v8a/sysflow"

mkdir -p "$ROOT/dist"
OUT="$ROOT/dist/sysflow-module-$VERSION.zip"
rm -f "$OUT"
( cd "$STAGE" && zip -qr "$OUT" . )
( cd "$ROOT/dist" && sha256sum "$(basename "$OUT")" > "$(basename "$OUT").sha256" )

echo "$OUT"
