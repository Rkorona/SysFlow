#!/system/bin/sh
# Late-start service: restore settings, launch the module-owned enforcement daemon and keep the
# battery transition watcher alive. Display transitions are handled inside the daemon (see
# core/sysflow-core/src/screen.rs); this script no longer polls the screen.

MODDIR=${0%/*}
BIN="$MODDIR/bin/sysflow"
DATA=/data/adb/sysflow

export SYSFLOW_DATA_DIR="$DATA"
export SYSFLOW_MODULE_DIR="$MODDIR"

mkdir -p "$DATA/log"

until [ "$(getprop sys.boot_completed)" = "1" ]; do
  sleep 2
done

[ -x "$BIN" ] || exit 0

# (Re)create the PATH shims at boot. `customize.sh` tries the same thing during install, but the
# manager's bin directory may not be mounted yet at that point, so that attempt can fail silently.
# Running here — after boot_completed, as root — is the reliable moment.
for BIN_DIR in /data/adb/ksu/bin /data/adb/ap/bin /data/adb/magisk; do
  [ -d "$BIN_DIR" ] || continue
  # A dangling link (previous install pointed at modules_update) also fails this test, so it is
  # replaced rather than left broken.
  [ -e "$BIN_DIR/sysflow" ] && continue
  ln -sf "$BIN" "$BIN_DIR/sysflow" 2>/dev/null || true
done

# The module owns the event source of truth; this write works even when the APK is not running.
$BIN event emit --module system --category lifecycle --type module_service_started \
  --severity info --title "SysFlow module service started" \
  --message "The Root module service reached the post-boot stage." \
  --dedupe-key module_service_started --dedupe-window-s 5 >/dev/null 2>&1 || true
# Keep the Root-owned journal bounded before the daemon starts; the APK applies its own Room retention.
$BIN event prune --retention-days 30 --max-events 5000 >/dev/null 2>&1 || true

# Restore state left by SoCFlow's removed performance-unlimit option before applying
# SysFlow profiles. This is a one-time compatibility cleanup, not a tuning feature.
if [ -r "$MODDIR/migrate-legacy.sh" ]; then
  sh "$MODDIR/migrate-legacy.sh" >> "$DATA/log/boot.log" 2>&1 || true
fi
if [ -r "$MODDIR/migrate-legacy-doze.sh" ]; then
  sh "$MODDIR/migrate-legacy-doze.sh" >> "$DATA/log/boot.log" 2>&1 || true
fi

# apply-boot also restores the independently configured WALT guard when ordinary boot apply
# is disabled. The user-configured delay is handled by the binary.
APPLY_RC=0
APPLY=$($BIN apply-boot 2>>"$DATA/log/boot.log") || APPLY_RC=$?
printf '%s\n' "$APPLY" >> "$DATA/log/boot.log"
if [ "$APPLY_RC" -ne 0 ]; then
  $BIN event emit --module system --category lifecycle --type boot_apply_crashed \
    --severity error --title "Boot settings apply failed" \
    --message "sysflow apply-boot exited with code $APPLY_RC." \
    --dedupe-key system.boot_apply_crashed --dedupe-window-s 60 >/dev/null 2>&1 || true
fi

APPLY_OK=0
case "$APPLY" in
  *'"errors":{}'*) APPLY_OK=1 ;;
esac

BOOT=$($BIN boot status 2>/dev/null)
case "$BOOT" in
  *'"enabled":true'*) ;;
  *) exit 0 ;;
esac

SAFE=$($BIN safe-mode status 2>/dev/null)
case "$SAFE" in
  *'"safe_mode":true'*|*'"rescue_marker":true'*)
    # Do not start automatic writers when safe/rescue mode is active.
    sleep 60
    exit 0
    ;;
esac

# Do not enforce settings after a failed boot-apply. The unconfirmed-boot counter will
# eventually enter safe mode instead of repeatedly forcing a potentially unstable config.
if [ "$APPLY_OK" = 1 ]; then
  $BIN daemon start >> "$DATA/log/boot.log" 2>&1
else
  echo "boot apply reported errors; daemon not started and this boot will remain unconfirmed" >> "$DATA/log/boot.log"
fi

# Battery transition watcher.
#
# The module is the event producer, so charging transitions are observed here rather than inferred by
# a client from dashboard snapshots. 60 s is the sampling cadence from the development plan: battery
# state changes are slow, and the loop must stay cheaper than the tuning daemon.
battery_loop() {
  BAT=/sys/class/power_supply/battery
  last_status=""
  low_fired=""
  hot_fired=""

  while true; do
    cap=""
    status=""
    temp=""
    { read -r cap < "$BAT/capacity"; } 2>/dev/null
    { read -r status < "$BAT/status"; } 2>/dev/null
    { read -r temp < "$BAT/temp"; } 2>/dev/null

    case "$status" in
      Charging)
        if [ "$last_status" != "Charging" ] && [ -n "$last_status" ]; then
          $BIN event emit --module battery --category charging --type charging_started \
            --severity notice --title "Charging started" \
            --message "The battery started charging." \
            --dedupe-key battery.charging.started --dedupe-window-s 30 >/dev/null 2>&1 || true
        fi
        # A new charge resets the low-battery latch so the next discharge reports again.
        low_fired=""
        ;;
      Full)
        if [ "$last_status" != "Full" ]; then
          $BIN event emit --module battery --category charging --type fully_charged \
            --severity notice --title "Battery fully charged" \
            --message "The battery reached full charge." \
            --metadata "{\"capacity\":${cap:-0}}" \
            --dedupe-key battery.fully_charged --dedupe-window-s 300 >/dev/null 2>&1 || true
        fi
        low_fired=""
        ;;
      Discharging|"Not charging")
        if [ "$last_status" = "Charging" ] || [ "$last_status" = "Full" ]; then
          $BIN event emit --module battery --category charging --type charging_stopped \
            --severity info --title "Charging stopped" \
            --message "The battery is no longer charging." \
            --dedupe-key battery.charging.stopped --dedupe-window-s 30 >/dev/null 2>&1 || true
        fi
        ;;
    esac

    # Low battery: once per discharge, only while actually on battery power.
    case "$status" in
      Discharging|"Not charging")
        if [ -n "$cap" ] && [ "$cap" -le 15 ] 2>/dev/null && [ -z "$low_fired" ]; then
          low_fired=1
          $BIN event emit --module battery --category power --type low_battery \
            --severity warning --title "Battery low" \
            --message "Battery dropped to ${cap}%." \
            --metadata "{\"battery_percent\":${cap}}" \
            --dedupe-key battery.low --dedupe-window-s 1800 >/dev/null 2>&1 || true
        fi
        ;;
    esac

    # Over-temperature: trip at 45.0 °C, re-arm below 43.0 °C to avoid flapping.
    if [ -n "$temp" ]; then
      if [ "$temp" -ge 450 ] 2>/dev/null && [ -z "$hot_fired" ]; then
        hot_fired=1
        $BIN event emit --module battery --category thermal --type battery_temp_high \
          --severity warning --title "Battery temperature high" \
          --message "Battery temperature reached $((temp / 10)) °C." \
          --metadata "{\"temperature_c\":$((temp / 10))}" \
          --dedupe-key battery.temp.high --dedupe-window-s 600 >/dev/null 2>&1 || true
      elif [ "$temp" -lt 430 ] 2>/dev/null; then
        hot_fired=""
      fi
    fi

    last_status=$status
    sleep 60
  done
}

# Battery transitions are reported by the module itself, so the APK never has to infer them.
battery_loop &

# Give the system a stability window before confirming this boot. Failed applies deliberately
# leave the boot counter unconfirmed; repeated failures isolate the profile into Recovery.
sleep 60
if [ "$APPLY_OK" = 1 ]; then
  $BIN boot-confirm >> "$DATA/log/boot.log" 2>&1
fi

# The tuning daemon is a separate detached process (it owns display transitions, drift repair and
# history sampling). The battery watcher loops forever in this script; keep the service alive for
# it so a terminated watcher does not leave a half-configured boot behind.
wait
