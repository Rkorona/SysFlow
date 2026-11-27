#!/bin/sh
# Check the repository file-mode policy.
#   tools/check-modes.sh
#
# Why this exists: almost every file was once committed as 100755 because of a bad umask. An exec
# bit is invisible in code review and turns every checkout into "mode change" noise, so the policy
# is checked mechanically instead of by eye. Only these real entry points may be executable:
#
#   100755  *.sh        shell scripts (build, module templates, tools, smoke tests)
#   100755  gradlew     Gradle wrapper launcher
#   100755  k.py        Python helper with a shebang
#   100644  everything else, including gradlew.bat: a Windows batch file needs no exec bit.
#
# The check reads `git ls-files -s` (the index), because the committed mode is what the policy is
# about; a local checkout with core.fileMode=false cannot be judged by its worktree bits.
# Exit status: 0 = policy holds, 1 = at least one path violates it (each one is listed).
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# The mode a tracked, repo-relative path must have.
expected_mode() {
  case "$1" in
    *.sh | gradlew | k.py) echo 100755 ;;
    *) echo 100644 ;;
  esac
}

# Collect violations instead of counting them: command substitution runs in a subshell, so the
# accumulated text is the result and no counter has to escape the loop.
violations="$(
  git ls-files -s | while read -r mode _hash _stage path; do
    case "$mode" in
      # 120000 = symlink, 160000 = submodule: neither carries a permission of its own.
      120000 | 160000) continue ;;
    esac
    expected="$(expected_mode "$path")"
    if [ "$mode" != "$expected" ]; then
      printf '  %s: expected %s, found %s\n' "$path" "$expected" "$mode"
    fi
  done
)"

if [ -n "$violations" ]; then
  echo "file-mode policy violation:" >&2
  printf '%s\n' "$violations" >&2
  cat >&2 <<'EOF'
fix each path with both commands, so the index and the worktree agree:
  chmod 644 <path> && git update-index --chmod=-x <path>   # should be 100644
  chmod +x  <path> && git update-index --chmod=+x <path>   # should be 100755
EOF
  exit 1
fi

count_mode() {
  git ls-files -s | awk -v mode="$1" '$1 == mode' | wc -l | tr -d ' '
}

echo "file modes OK: $(count_mode 100755) executable, $(count_mode 100644) non-executable"
