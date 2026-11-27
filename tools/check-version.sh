#!/bin/sh
# Verify that the module version has a single source of truth.
#
#   - module/module.prop `version` must match README's "Current module version: `vX.Y.Z`" line
#   - module/module.prop `versionCode` must equal MAJOR*10000 + MINOR*100 + PATCH (v0.3.3 -> 303)
#
# core/Cargo.toml tracks an independent binary/protocol version line and is intentionally not
# compared here (see docs/protocol.md §3.1).
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROP="$ROOT/module/module.prop"
README="$ROOT/README.md"

fail() {
  echo "version check failed: $*" >&2
  exit 1
}

VERSION="$(sed -n 's/^version=//p' "$PROP" | head -n 1)"
VERSION_CODE="$(sed -n 's/^versionCode=//p' "$PROP" | head -n 1)"
[ -n "$VERSION" ] || fail "module/module.prop has no version= entry"
[ -n "$VERSION_CODE" ] || fail "module/module.prop has no versionCode= entry"

README_VERSION="$(sed -n 's/^Current module version: `\(v[0-9][0-9.]*\)`.*$/\1/p' "$README" | head -n 1)"
[ -n "$README_VERSION" ] || fail 'README.md has no "Current module version: `vX.Y.Z`" line'
[ "$README_VERSION" = "$VERSION" ] || \
  fail "README version '$README_VERSION' does not match module.prop version '$VERSION'"

case "$VERSION" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "module version '$VERSION' is not of the form vMAJOR.MINOR.PATCH" ;;
esac

NUM="${VERSION#v}"
MAJOR="${NUM%%.*}"
REST="${NUM#*.}"
MINOR="${REST%%.*}"
PATCH="${REST#*.}"

case "$MAJOR$MINOR$PATCH" in
  ''|*[!0-9]*) fail "module version '$VERSION' is not numeric MAJOR.MINOR.PATCH" ;;
esac
case "$PATCH" in
  *.*) fail "module version '$VERSION' has more than three components" ;;
esac

case "$VERSION_CODE" in
  ''|*[!0-9]*) fail "versionCode '$VERSION_CODE' is not numeric" ;;
esac

EXPECTED=$((MAJOR * 10000 + MINOR * 100 + PATCH))
[ "$VERSION_CODE" -eq "$EXPECTED" ] || \
  fail "versionCode $VERSION_CODE does not match $VERSION (expected $EXPECTED)"

README_LINE="$(grep -n '^Current module version: ' "$README" | head -n 1 | cut -d: -f1)"
echo "module version $VERSION (versionCode $VERSION_CODE) consistent with README.md:$README_LINE"
