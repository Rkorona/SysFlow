#!/bin/sh
# Verify that every copy of the wire-protocol version agrees with the Rust runtime constant.
#
# The policy behind this (when to bump, what a bump requires) lives in `docs/protocol.md` §5. This
# script is only a cheap consistency gate: it fails when a bump — or a revert — touched some copies
# but not all of them.
#
# Copies checked:
#   core/sysflow-core/src/envelope.rs              runtime truth (`pub const PROTOCOL_VERSION`,
#                                                  also used for the envelope `v` field)
#   core/sysflow-types/src/main.rs                 generator literals for the Kotlin AND TypeScript
#                                                  output (deliberately not derived from envelope.rs:
#                                                  sysflow-types has no core dependency)
#   app/src/main/kotlin/.../Protocol.kt            generated Kotlin constant, committed
#   module/webroot/app.js                          rescue-console constant
#   docs/protocol.md                               every `{"v":N}` and `"protocol":N` example
#
# The `{"v":N}` examples are checked too because the envelope carries the same number; a partial docs
# update would otherwise teach clients the wrong value. Matching is limited to those two JSON tokens
# on purpose, so prose about version mismatches (e.g. "模块 1，应用 2") is not matched.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENVELOPE="$ROOT/core/sysflow-core/src/envelope.rs"
TYPES="$ROOT/core/sysflow-types/src/main.rs"
KOTLIN="$ROOT/app/src/main/kotlin/io/sysflow/core/model/Protocol.kt"
WEBUI="$ROOT/module/webroot/app.js"
DOCS="$ROOT/docs/protocol.md"

fail() {
  echo "protocol version check failed: $*" >&2
  exit 1
}

# read_value <file> <sed-script> <label>: first match printed on its own line.
read_value() {
  [ -f "$1" ] || fail "${1#"$ROOT/"} is missing"
  _value="$(sed -n "$2" "$1" | head -n 1)"
  [ -n "$_value" ] || fail "$3 not found in ${1#"$ROOT/"}"
  printf '%s\n' "$_value"
}

# expect <label> <value>
expect() {
  [ "$2" = "$RUST" ] || \
    fail "$1 is '$2' but core/sysflow-core/src/envelope.rs says '$RUST'"
}

RUST="$(read_value "$ENVELOPE" \
  's/^pub const PROTOCOL_VERSION: u32 = \([0-9][0-9]*\);$/\1/p' \
  'pub const PROTOCOL_VERSION')"

KOTLIN_VALUE="$(read_value "$KOTLIN" \
  's/^const val PROTOCOL_VERSION: Int = \([0-9][0-9]*\)$/\1/p' \
  'generated Kotlin PROTOCOL_VERSION')"
expect "generated Kotlin PROTOCOL_VERSION" "$KOTLIN_VALUE"

GENERATOR_KOTLIN="$(read_value "$TYPES" \
  's/^[[:space:]]*const val PROTOCOL_VERSION: Int = \([0-9][0-9]*\).*$/\1/p' \
  'generator Kotlin literal')"
expect "core/sysflow-types/src/main.rs Kotlin literal" "$GENERATOR_KOTLIN"

GENERATOR_TS="$(read_value "$TYPES" \
  's/.*export const PROTOCOL_VERSION = \([0-9][0-9]*\);.*$/\1/p' \
  'generator TypeScript literal')"
expect "core/sysflow-types/src/main.rs TypeScript literal" "$GENERATOR_TS"

WEBUI_VALUE="$(read_value "$WEBUI" \
  's/^const PROTOCOL_VERSION = \([0-9][0-9]*\);$/\1/p' \
  'rescue-console PROTOCOL_VERSION')"
expect "module/webroot/app.js PROTOCOL_VERSION" "$WEBUI_VALUE"

# Envelope examples: `{"v":1, ...}` and `"protocol":1` anywhere in the protocol document.
V_EXAMPLES="$(grep -o '{"v":[[:space:]]*[0-9][0-9]*' "$DOCS" || true)"
P_EXAMPLES="$(grep -o '"protocol":[[:space:]]*[0-9][0-9]*' "$DOCS" || true)"
[ -n "$V_EXAMPLES" ] || fail 'docs/protocol.md has no `{"v":N}` envelope example to verify'
[ -n "$P_EXAMPLES" ] || fail 'docs/protocol.md has no `"protocol":N` example to verify'

V_COUNT="$(printf '%s\n' "$V_EXAMPLES" | wc -l | tr -d ' ')"
P_COUNT="$(printf '%s\n' "$P_EXAMPLES" | wc -l | tr -d ' ')"

for _example in $(printf '%s\n' "$V_EXAMPLES" | sed 's/[^0-9]//g'); do
  expect 'docs/protocol.md `{"v":N}` example' "$_example"
done
for _example in $(printf '%s\n' "$P_EXAMPLES" | sed 's/[^0-9]//g'); do
  expect 'docs/protocol.md `"protocol":N` example' "$_example"
done

echo "protocol version $RUST consistent: envelope.rs, Protocol.kt, generator (Kotlin+TS),"
echo "  module/webroot/app.js, docs/protocol.md ($V_COUNT envelope + $P_COUNT protocol examples)"
