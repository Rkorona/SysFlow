#!/system/bin/sh
# One-time compatibility cleanup for SoCFlow's retired screen-off Doze worker.
# SysFlow does not ship the old governor. Stop it safely, undo only power-state changes
# that SoCFlow recorded as its own, and remove the obsolete worker state.

LEGACY_DATA=${SYSFLOW_LEGACY_DATA_DIR:-/data/adb/soc_flow}
PROC_ROOT=${SYSFLOW_PROC_ROOT:-/proc}
PID_FILE="$LEGACY_DATA/sleep.pid"
STATE_FILE="$LEGACY_DATA/sleep-state.conf"

# Avoid touching Android power state when no retired feature files remain.
if [ ! -e "$PID_FILE" ] && [ ! -e "$STATE_FILE" ] && \
   [ ! -e "$LEGACY_DATA/sleep.conf" ] && [ ! -e "$LEGACY_DATA/sleep-offstart" ] && \
   [ ! -e "$LEGACY_DATA/.sleep-lock" ] && [ ! -e "$LEGACY_DATA/.sleep-oplock" ]; then
  exit 0
fi

is_legacy_worker() {
  _pid=$1
  case "$_pid" in ''|*[!0-9]*) return 1 ;; esac
  [ -r "$PROC_ROOT/$_pid/cmdline" ] || return 1
  tr '\000' ' ' < "$PROC_ROOT/$_pid/cmdline" 2>/dev/null | grep -q 'sleep-governor.sh'
}

# Let the old worker's EXIT trap run first. Never signal a PID unless its cmdline
# still identifies the retired worker (the PID may have been reused).
_pid=""
[ -r "$PID_FILE" ] && { read -r _pid < "$PID_FILE"; }
if is_legacy_worker "$_pid"; then
  kill -TERM "$_pid" 2>/dev/null
  _wait=0
  while [ "$_wait" -lt 15 ] && is_legacy_worker "$_pid"; do
    sleep 1
    _wait=$((_wait + 1))
  done
  if is_legacy_worker "$_pid"; then
    kill -KILL "$_pid" 2>/dev/null
    sleep 1
  fi
fi

active=0
low_power_before=0
low_power_changed=0
doze_forced=0
if [ -r "$STATE_FILE" ]; then
  while IFS='=' read -r key value; do
    case "$key" in
      active) active=$value ;;
      low_power_before) low_power_before=$value ;;
      low_power_changed) low_power_changed=$value ;;
      doze_forced) doze_forced=$value ;;
    esac
  done < "$STATE_FILE"
fi

if [ "$active" = 1 ]; then
  if [ "$doze_forced" = 1 ] && command -v cmd >/dev/null 2>&1; then
    cmd deviceidle unforce >/dev/null 2>&1
  fi
  if [ "$low_power_changed" = 1 ] && [ "$low_power_before" = 0 ] && \
      command -v settings >/dev/null 2>&1 && command -v cmd >/dev/null 2>&1 && \
      [ "$(settings get global low_power 2>/dev/null)" = 1 ]; then
    cmd power set-mode 0 >/dev/null 2>&1
  fi
fi

rm -f \
  "$LEGACY_DATA/sleep.conf" "$LEGACY_DATA/sleep.conf.bad" \
  "$STATE_FILE" "$LEGACY_DATA/sleep-offstart" "$PID_FILE" "$LEGACY_DATA/sleep-governor.log"
rm -rf "$LEGACY_DATA/.sleep-lock" "$LEGACY_DATA/.sleep-oplock"
exit 0
