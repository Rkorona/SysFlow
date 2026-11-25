#!/system/bin/sh
# One-time migration cleanup for SoCFlow's removed performance-unlimit feature.
# The feature is not carried into SysFlow. This only restores the value that the old
# module saved before it changed msm_performance, then restarts perfsdkserver if the
# old module had stopped it. The backup is retained when the sysfs node is unavailable
# or the write cannot be verified, so a later boot can retry safely.

LEGACY_DATA=${SYSFLOW_LEGACY_DATA_DIR:-/data/adb/soc_flow}
DATA=${SYSFLOW_DATA_DIR:-/data/adb/sysflow}
MP_ROOT=${SYSFLOW_MSM_PERF_DIR:-/sys/kernel/msm_performance/parameters}
BACKUP="$LEGACY_DATA/msmperf-original.conf"
FLAG="$LEGACY_DATA/unlimit"
LOG_DIR="$DATA/log"
LOG_FILE="$LOG_DIR/legacy-migration.log"

mkdir -p "$LOG_DIR" 2>/dev/null
log() {
  printf '%s\n' "$*"
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$*" >> "$LOG_FILE" 2>/dev/null
}

[ -f "$FLAG" ] || [ -s "$BACKUP" ] || exit 0

NODE="$MP_ROOT/cpu_max_freq"
if [ ! -e "$NODE" ] && [ "$MP_ROOT" = /sys/kernel/msm_performance/parameters ]; then
  ALT=/sys/module/msm_performance/parameters/cpu_max_freq
  [ -e "$ALT" ] && NODE=$ALT
fi

RESTORED=1
if [ -s "$BACKUP" ]; then
  OLD=""
  IFS= read -r OLD < "$BACKUP" || true
  case "$OLD" in
    ''|*[!0-9]*)
      RESTORED=0
      log "legacy cleanup: invalid msmperf backup; retained for manual inspection"
      ;;
    *)
      if [ -e "$NODE" ]; then
        if printf '%s\n' "$OLD" > "$NODE" 2>/dev/null; then
          CURRENT=""
          IFS= read -r CURRENT < "$NODE" 2>/dev/null || true
          if [ "$CURRENT" = "$OLD" ]; then
            rm -f "$BACKUP"
            log "legacy cleanup: restored msm_performance cpu_max_freq=$OLD"
          else
            RESTORED=0
            log "legacy cleanup: cpu_max_freq read-back mismatch; backup retained (wanted $OLD, got ${CURRENT:-unreadable})"
          fi
        else
          RESTORED=0
          log "legacy cleanup: failed to restore $NODE; backup retained"
        fi
      else
        RESTORED=0
        log "legacy cleanup: msm_performance cpu_max_freq node not found; backup retained"
      fi
      ;;
  esac
fi

# Match the old module's compatibility behavior: restart the service it stopped,
# but never issue a service command unless Android reports it is currently stopped.
if command -v getprop >/dev/null 2>&1 && [ "$(getprop init.svc.perfsdkserver 2>/dev/null)" = stopped ]; then
  if command -v start >/dev/null 2>&1; then
    if start perfsdkserver >/dev/null 2>&1; then
      log "legacy cleanup: restarted perfsdkserver"
    else
      log "legacy cleanup: failed to restart perfsdkserver"
    fi
  else
    log "legacy cleanup: perfsdkserver is stopped but start command is unavailable"
  fi
fi

if [ "$RESTORED" = 1 ]; then
  rm -f "$FLAG"
  log "legacy cleanup: completed"
  exit 0
fi

# Keep the marker and backup in place when restoration was incomplete. The script is
# idempotent and can retry on the next module service start.
exit 1
