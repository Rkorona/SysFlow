#!/bin/sh
# Pack the SysFlow module zip.
#   tools/pack-module.sh [path-to-arm64-binary]
# Default binary: core/target/aarch64-linux-android/release/sysflow
# Output: dist/sysflow-module-v<version>.zip (+ .sha256)
#
# The archive is reproducible: entries are sorted, timestamps are pinned (SOURCE_DATE_EPOCH, falling
# back to the last commit time), permissions are normalized and no directory entries are written, so
# the same binary at the same commit yields the same SHA256. python3 is the only archive writer,
# which keeps local and CI output byte-identical and removes the `zip` dependency.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/core/target/aarch64-linux-android/release/sysflow}"
[ -f "$BIN" ] || { echo "binary not found: $BIN" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || { echo "missing required command: $1" >&2; exit 1; }
}
need_cmd python3
need_cmd sha256sum

VERSION="$(sed -n 's/^version=//p' "$ROOT/module/module.prop" | head -n 1)"
[ -n "$VERSION" ] || { echo "no version in module.prop" >&2; exit 1; }

[ -f "$ROOT/module/webroot/index.html" ] || {
  echo "module/webroot/index.html is missing; the WebUI is a static asset under module/webroot" >&2
  exit 1
}

# Pinned archive timestamp: honour SOURCE_DATE_EPOCH when set, otherwise use the checkout's commit
# time. ZIP timestamps cannot represent dates before 1980-01-01 (315532800), so clamp to it.
EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || echo 315532800)}"
case "$EPOCH" in
  ''|*[!0-9]*) echo "invalid SOURCE_DATE_EPOCH: $EPOCH" >&2; exit 1 ;;
esac
[ "$EPOCH" -ge 315532800 ] || EPOCH=315532800

umask 022
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

python3 - "$STAGE" "$OUT" "$EPOCH" <<'PY'
import os
import sys
import time
import zipfile

stage, out, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
# ZIP stores DOS timestamps with two-second granularity; floor so the pinned value is exact.
timestamp = time.gmtime(epoch - (epoch % 2))[:6]


def mode_for(rel):
    # The installer runs the shell scripts and the CLI binary; everything else is plain data.
    if rel.endswith(".sh") or rel == "bin/arm64-v8a/sysflow":
        return 0o755
    return 0o644


entries = []
for root, dirs, files in os.walk(stage):
    dirs.sort()
    for name in files:
        path = os.path.join(root, name)
        entries.append(os.path.relpath(path, stage).replace(os.sep, "/"))
# Byte order == LC_ALL=C for these ASCII paths; no locale-dependent sorting is involved.
entries.sort()

with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
    for rel in entries:
        path = os.path.join(stage, rel)
        info = zipfile.ZipInfo(rel, date_time=timestamp)
        info.compress_type = zipfile.ZIP_DEFLATED
        info.create_system = 3  # Unix, so external_attr carries the permission bits
        info.external_attr = (mode_for(rel) << 16) | 0o100000
        with open(path, "rb") as fh:
            zf.writestr(info, fh.read())

with zipfile.ZipFile(out) as zf:
    broken = zf.testzip()
if broken is not None:
    raise SystemExit("corrupt archive entry: " + broken)
PY

( cd "$ROOT/dist" && sha256sum "$(basename "$OUT")" > "$(basename "$OUT").sha256" )

echo "$OUT"
