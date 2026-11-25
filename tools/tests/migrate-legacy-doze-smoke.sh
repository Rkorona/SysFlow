#!/bin/sh
set -eu
ROOT="$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/module/migrate-legacy-doze.sh"
T="$(mktemp -d "${TMPDIR:-/tmp}/sysflow-legacy-doze.XXXXXX")"
trap 'rm -rf "$T"' EXIT INT TERM HUP
LEGACY="$T/legacy"
DATA="$T/sysflow"
BIN="$T/bin"
CALLS="$T/calls"
PROC="$T/proc"
mkdir -p "$LEGACY" "$DATA" "$BIN" "$PROC"
: > "$CALLS"

cat > "$BIN/settings" <<'MOCK'
#!/bin/sh
[ "$*" = 'get global low_power' ] && { echo "${MOCK_LOW_POWER:-1}"; exit 0; }
echo "unexpected settings $*" >&2
exit 1
MOCK
cat > "$BIN/cmd" <<EOF2
#!/bin/sh
printf '%s\\n' "\$*" >> "$CALLS"
EOF2
chmod +x "$BIN/settings" "$BIN/cmd"

# Restore only the power-state changes recorded as module-owned by the old worker.
printf 'active=1\nlow_power_before=0\nlow_power_changed=1\ndoze_forced=1\n' > "$LEGACY/sleep-state.conf"
: > "$LEGACY/sleep.conf"
: > "$LEGACY/sleep.conf.bad"
: > "$LEGACY/sleep-offstart"
: > "$LEGACY/sleep.pid"
mkdir "$LEGACY/.sleep-lock" "$LEGACY/.sleep-oplock"
PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_PROC_ROOT="$PROC" sh "$SCRIPT"
grep -qx 'deviceidle unforce' "$CALLS" || { echo 'FAIL: legacy Doze was not unforced' >&2; exit 1; }
grep -qx 'power set-mode 0' "$CALLS" || { echo 'FAIL: module-owned low-power mode was not restored' >&2; exit 1; }
for f in sleep-state.conf sleep.conf sleep.conf.bad sleep-offstart sleep.pid sleep-governor.log; do
  [ ! -e "$LEGACY/$f" ] || { echo "FAIL: obsolete $f remains" >&2; exit 1; }
done
[ ! -e "$LEGACY/.sleep-lock" ] && [ ! -e "$LEGACY/.sleep-oplock" ] || {
  echo 'FAIL: obsolete locks remain' >&2; exit 1;
}

# Preserve low-power mode if it was already enabled before the old worker forced it.
: > "$CALLS"
printf 'active=1\nlow_power_before=1\nlow_power_changed=0\ndoze_forced=0\n' > "$LEGACY/sleep-state.conf"
PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_PROC_ROOT="$PROC" MOCK_LOW_POWER=1 sh "$SCRIPT"
[ ! -s "$CALLS" ] || { echo 'FAIL: pre-existing low-power mode was changed' >&2; exit 1; }

# Orphaned lock directories are cleaned even when the main state/config files are gone.
mkdir "$LEGACY/.sleep-lock" "$LEGACY/.sleep-oplock"
PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_PROC_ROOT="$PROC" sh "$SCRIPT"
[ ! -e "$LEGACY/.sleep-lock" ] && [ ! -e "$LEGACY/.sleep-oplock" ] || {
  echo 'FAIL: orphan legacy locks remain' >&2; exit 1;
}

# No legacy files is a no-op.
PATH="$BIN:$PATH" SYSFLOW_LEGACY_DATA_DIR="$LEGACY" SYSFLOW_PROC_ROOT="$PROC" sh "$SCRIPT"

echo 'legacy Doze migration smoke: OK'
