#!/bin/sh
set -eu
ROOT="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/module/migrate-legacy.sh"
T="$(mktemp -d "${TMPDIR:-/tmp}/sysflow-legacy-migration.XXXXXX")"
trap 'rm -rf "$T"' EXIT INT TERM HUP

LEGACY="$T/legacy"
DATA="$T/sysflow"
MP="$T/msm_performance"
BIN="$T/bin"
CALLS="$T/start.calls"
mkdir -p "$LEGACY" "$DATA" "$MP" "$BIN"
printf '123456\n' > "$LEGACY/msmperf-original.conf"
printf '999999\n' > "$MP/cpu_max_freq"
: > "$LEGACY/unlimit"
: > "$CALLS"

cat > "$BIN/getprop" <<'MOCK'
#!/bin/sh
[ "${1:-}" = init.svc.perfsdkserver ] && echo stopped
MOCK
cat > "$BIN/start" <<EOF2
#!/bin/sh
printf '%s\\n' "\$*" >> "$CALLS"
EOF2
chmod +x "$BIN/getprop" "$BIN/start"

PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_DATA_DIR="$DATA" \
  SYSFLOW_MSM_PERF_DIR="$MP" sh "$SCRIPT" >/dev/null

[ "$(cat "$MP/cpu_max_freq")" = 123456 ] || { echo 'FAIL: legacy CPU max was not restored' >&2; exit 1; }
[ ! -e "$LEGACY/unlimit" ] || { echo 'FAIL: legacy marker not removed' >&2; exit 1; }
[ ! -e "$LEGACY/msmperf-original.conf" ] || { echo 'FAIL: restored backup not removed' >&2; exit 1; }
grep -qx 'perfsdkserver' "$CALLS" || { echo 'FAIL: stopped performance service was not restarted' >&2; exit 1; }

# If the sysfs target is absent, preserve both marker and backup for another attempt.
printf '654321\n' > "$LEGACY/msmperf-original.conf"
: > "$LEGACY/unlimit"
if PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_DATA_DIR="$DATA" \
  SYSFLOW_MSM_PERF_DIR="$T/missing" sh "$SCRIPT" >/dev/null; then
  echo 'FAIL: missing legacy sysfs node should report an incomplete migration' >&2
  exit 1
fi
[ -e "$LEGACY/unlimit" ] && [ -s "$LEGACY/msmperf-original.conf" ] || {
  echo 'FAIL: incomplete migration did not preserve its retry backup' >&2
  exit 1
}

echo 'legacy migration smoke: OK'
