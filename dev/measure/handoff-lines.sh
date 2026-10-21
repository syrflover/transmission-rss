#!/usr/bin/env bash
# Lines of the code that hands commands from the web to the worker, the
# yardstick of the lighter-structure change (tickets 0144 and 0161).
#
#   dev/measure/handoff-lines.sh [<rev>]
#
# <rev> defaults to HEAD; the committed files are counted, not the working
# tree. A count is every line of the `.rs` files (`git grep -c ''`), comments
# and blank lines included, as the survey of 2026-10-21 counted
# (`5cdae9e`: trss-core 3,873, trss-worker 10,503 with 7,059 in tests/).
#
# trss-core's part is its commands, queue, wake, heartbeat, lock and
# folder_locks code; a path that is gone counts 0. Each of trss-web,
# trss-worker and trss that exists at <rev> is split into src/ without its
# test files, src/'s test files (`tests.rs` and files under a `tests/`
# folder) and tests/. Tests inside a `mod tests` of another file count as
# src/.
set -euo pipefail
[[ ${1:-} == -h || ${1:-} == --help ]] && { sed -n '5p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }
rev=${1:-HEAD}
cd "$(git rev-parse --show-toplevel)"

# The line count of every `.rs` file under the given paths at $rev, one
# `<path> <lines>` per line.
counts() {
  { git grep -c '' "$rev" -- "$@" || true; } | sed -nE "s#^[^:]+:(.+\.rs):([0-9]+)\$#\1 \2#p"
}
sum() { awk '{s += $2} END {print s + 0}'; }

echo "rev $(git rev-parse --short "$rev")"
echo "trss-core handoff"
total=0
for p in commands queue.rs wake.rs heartbeat.rs lock.rs folder_locks.rs; do
  n=$(counts "crates/trss-core/src/$p" | sum)
  total=$((total + n))
  printf '  %-16s %6d\n' "$p" "$n"
done
printf '  %-16s %6d\n' total "$total"

for c in trss-web trss-worker trss; do
  all=$(counts "crates/$c")
  [[ -z $all ]] && continue
  src_tests=$(grep -E "^crates/$c/src/(.*/)?(tests\.rs|tests/[^ ]*) " <<<"$all" | sum || true)
  src=$(grep -E "^crates/$c/src/" <<<"$all" | sum)
  tests=$(grep -E "^crates/$c/tests/" <<<"$all" | sum || true)
  printf '%-12s src %6d  src-tests %6d  tests %6d  all %6d\n' \
    "$c" $((src - src_tests)) "$src_tests" "$tests" "$(sum <<<"$all")"
done
