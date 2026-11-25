#!/system/bin/sh
# Restore every default SysFlow changed, then remove its data.

MODDIR=${0%/*}
BIN="$MODDIR/bin/sysflow"

# Restore state left by retired SoCFlow features before removing the module.
[ -r "$MODDIR/migrate-legacy-doze.sh" ] && sh "$MODDIR/migrate-legacy-doze.sh" >/dev/null 2>&1 || true
[ -r "$MODDIR/migrate-legacy.sh" ] && SYSFLOW_DATA_DIR=/data/adb/sysflow sh "$MODDIR/migrate-legacy.sh" >/dev/null 2>&1 || true

if [ -x "$BIN" ]; then
  SYSFLOW_DATA_DIR=/data/adb/sysflow "$BIN" daemon stop > /dev/null 2>&1
  SYSFLOW_DATA_DIR=/data/adb/sysflow "$BIN" reset --all > /dev/null 2>&1
fi

# Remove the PATH shims created at install time.
for BIN_DIR in /data/adb/ksu/bin /data/adb/ap/bin /data/adb/magisk; do
  [ -L "$BIN_DIR/sysflow" ] && rm -f "$BIN_DIR/sysflow" 2>/dev/null
done

rm -rf /data/adb/sysflow
