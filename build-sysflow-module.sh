#!/usr/bin/env bash
# Build SysFlow ARM64 binary, WebUI and KernelSU/Magisk module ZIP.
# Run from anywhere: bash /path/to/SysFlow/build-sysflow-module.sh
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '\n\033[1;36m[SysFlow]\033[0m %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"; }

usage() {
  cat <<'USAGE'
SysFlow module build

Usage:
  ./build-sysflow-module.sh
  ./build-sysflow-module.sh --help

Builds:
  1. Rust ARM64 release binary
  2. WebUI assets
  3. KernelSU / Magisk module ZIP and SHA256 checksum

Does NOT build the Android APK.

Required tools:
  Rust (rustup + cargo), python3, sha256sum
  Android NDK (NDK path/configuration required by cargo-ndk)

The WebUI is plain HTML/CSS/JS under module/webroot and needs no build step.

Output:
  dist/sysflow-module-<version>.zip
  dist/sysflow-module-<version>.zip.sha256
USAGE
}

case "${1:-}" in
  "") ;;
  --help|-h) usage; exit 0 ;;
  *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
esac

[[ -d "$ROOT/core" ]] || die "Cannot find core/ under project root: $ROOT"
[[ -f "$ROOT/module/module.prop" ]] || die "Cannot find module/module.prop"
[[ -f "$ROOT/tools/pack-module.sh" ]] || die "Cannot find tools/pack-module.sh"

log "Checking build tools"
need_cmd rustup
need_cmd cargo
need_cmd python3
need_cmd sha256sum

if ! command -v cargo-ndk >/dev/null 2>&1; then
  log "cargo-ndk not found; installing"
  cargo install cargo-ndk
fi

if ! rustup target list --installed | grep -qx 'aarch64-linux-android'; then
  log "Installing Rust target aarch64-linux-android"
  rustup target add aarch64-linux-android
fi

log "Building Rust ARM64 release binary"
(cd "$ROOT/core" && cargo ndk -t arm64-v8a build --release -p sysflow-cli)
BIN="$ROOT/core/target/aarch64-linux-android/release/sysflow"
[[ -s "$BIN" ]] || die "Expected binary was not generated: $BIN"

# The WebUI is a static rescue console (module/webroot/{index.html,app.js,app.css}): no framework,
# no build step, no npm. Nothing to compile here; just make sure it is present.
log "Checking WebUI"
[[ -s "$ROOT/module/webroot/index.html" ]] || die "WebUI missing: module/webroot/index.html"
[[ -s "$ROOT/module/webroot/app.js" ]] || die "WebUI missing: module/webroot/app.js"

log "Packaging KernelSU / Magisk module"
sh "$ROOT/tools/pack-module.sh" "$BIN"

VERSION="$(sed -n 's/^version=//p' "$ROOT/module/module.prop" | head -n 1)"
[[ -n "$VERSION" ]] || die "Could not read module version from module/module.prop"
ZIP="$ROOT/dist/sysflow-module-${VERSION}.zip"
[[ -s "$ZIP" ]] || die "Module ZIP was not generated: $ZIP"

log "Verifying SHA256 checksum"
if [[ -s "${ZIP}.sha256" ]]; then
  (cd "$(dirname "$ZIP")" && sha256sum -c "$(basename "${ZIP}.sha256")")
else
  (cd "$(dirname "$ZIP")" && sha256sum "$(basename "$ZIP")" > "$(basename "${ZIP}.sha256")")
  (cd "$(dirname "$ZIP")" && sha256sum -c "$(basename "${ZIP}.sha256")")
fi

log "Build completed successfully"
printf '\nModule ZIP: %s\nSHA256:     %s\n' "$ZIP" "${ZIP}.sha256"
