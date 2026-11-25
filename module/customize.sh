#!/system/bin/sh
# SysFlow installer (Magisk / KernelSU / APatch). The manager extracts the zip
# into $MODPATH before running this script (no SKIPUNZIP).

DATA_DIR=/data/adb/sysflow

ui_print "- SysFlow: installing"

case "$ARCH" in
  arm64) BIN_ARCH=arm64-v8a ;;
  *) abort "! Unsupported architecture: $ARCH (SysFlow currently supports arm64 only)" ;;
esac

if [ ! -f "$MODPATH/bin/$BIN_ARCH/sysflow" ]; then
  abort "! Missing binary for $BIN_ARCH inside the zip"
fi

# keep only the binary for this device
mv "$MODPATH/bin/$BIN_ARCH/sysflow" "$MODPATH/sysflow.bin" || abort "! Failed to place the binary"
rm -rf "$MODPATH/bin"
mkdir -p "$MODPATH/bin"
mv "$MODPATH/sysflow.bin" "$MODPATH/bin/sysflow"

# user data survives updates; only create what is missing
mkdir -p "$DATA_DIR/log"

set_perm_recursive "$MODPATH" 0 0 0755 0644
set_perm "$MODPATH/bin/sysflow" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755
set_perm "$MODPATH/migrate-legacy.sh" 0 0 0755
set_perm "$MODPATH/migrate-legacy-doze.sh" 0 0 0755

# Expose `sysflow` on the manager's PATH so the CLI instructions in the docs work verbatim on a
# device shell.
#
# The link must point at the FINAL module path, not at $MODPATH: the manager stages the module under
# /data/adb/modules_update/<id> during install and moves it to /data/adb/modules/<id> on reboot, so a
# link into the staging directory is dangling the moment the device comes back up.
MODULE_FINAL="/data/adb/modules/$(basename "$MODPATH")"
for BIN_DIR in /data/adb/ksu/bin /data/adb/ap/bin /data/adb/magisk; do
  [ -d "$BIN_DIR" ] || continue
  ln -sf "$MODULE_FINAL/bin/sysflow" "$BIN_DIR/sysflow" 2>/dev/null || true
done

# Stop any daemon still running from the previous install. Its process keeps the old binary's inode
# alive, so new features (history sampling) would silently not exist until the next reboot.
INSTALLED="/data/adb/modules/sysflow/bin/sysflow"
if [ -x "$INSTALLED" ]; then
  SYSFLOW_DATA_DIR="$DATA_DIR" "$INSTALLED" daemon stop >/dev/null 2>&1 || true
fi

# install-time self check: informational only, never blocks the install
ui_print "- Running capability probe"
if CAPS="$(SYSFLOW_DATA_DIR=$DATA_DIR "$MODPATH/bin/sysflow" caps 2>&1)"; then
  ui_print "  $CAPS"
else
  ui_print "! Capability probe failed (the module is still installed):"
  ui_print "  $CAPS"
fi

ui_print "- Done. Open the SysFlow app, or the module's WebUI, to continue."
