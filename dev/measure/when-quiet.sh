#!/usr/bin/env bash
# Runs a measurement when no other build or test uses the machine, and says
# whether one started while it ran (ticket 0161).
#
#   dev/measure/when-quiet.sh <log> <command> [<arg>...]
#
# Another build is a cargo or rustc process, or a binary under some
# target/debug/deps/, whose working directory is outside this repository, or
# a cargo here that is not `cargo test` (rust-analyzer's check). The script
# waits until there was none for 90 seconds (QUIET_SECONDS), runs the command, and looks once
# a second while it runs; each one it sees goes to <log>. It exits with the
# command's status, or 3 when it saw one, so that the measurement can be run
# again. A process that lives less than a second can go unseen.
set -uo pipefail
[[ $# -ge 2 ]] || { sed -n '5p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
log=$1
shift
repo=$(git rev-parse --show-toplevel)

# Prints each other build that runs now.
others() {
  local pid cwd cmd
  for pid in $(pgrep -x 'cargo|rustc'; pgrep -f '/target/debug/deps/'); do
    cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null) || continue
    cmd=$(tr '\0' ' ' 2>/dev/null < "/proc/$pid/cmdline" | cut -c1-160) || continue
    if [[ $cwd == "$repo"* ]]; then
      [[ $cmd =~ (^|/)cargo\ test || ! $cmd =~ (^|/)cargo\  ]] && continue
    fi
    printf '%s %s %s %s\n' "$(date +%T)" "$pid" "$cwd" "$cmd"
  done
}

quiet=${QUIET_SECONDS:-90}
quiet_since=$(date +%s)
while (($(date +%s) - quiet_since < quiet)); do
  [[ -n $(others) ]] && quiet_since=$(date +%s)
  sleep 1
done

"$@" &
cmd=$!
seen=0
while kill -0 "$cmd" 2>/dev/null; do
  found=$(others)
  if [[ -n $found ]]; then
    printf '%s\n' "$found" >> "$log"
    seen=1
  fi
  sleep 1
done
wait "$cmd"
status=$?
((seen)) && exit 3
exit "$status"
