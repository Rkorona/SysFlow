#!/bin/sh
# Generate the Kotlin protocol models from the Rust source of truth.
#   tools/gen-types.sh
# Output:
#   app/src/main/kotlin/io/sysflow/core/model/Protocol.kt
#
# The module WebUI is plain HTML/JS and consumes the JSON envelope directly, so no TypeScript model
# is generated any more. The generator still supports --ts if a typed client is reintroduced.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

cd "$ROOT/core"
cargo run -q -p sysflow-types -- \
  --kotlin "$ROOT/app/src/main/kotlin/io/sysflow/core/model/Protocol.kt"
