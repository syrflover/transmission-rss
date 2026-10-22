#!/usr/bin/env bash
# The workspace's test run time, measured as the lighter-structure baseline
# measured it (tickets 0144 and 0160).
#
#   dev/measure/test-runs.sh <out dir> [<runs>]
#
# Builds once with
#
#   cargo test --locked --workspace --no-run -j 4
#
# then runs `cargo test --locked --workspace -j 4` <runs> times (default 2)
# without changing anything, so that every run finds the binaries built.
# Before each build or run it stops if another cargo, rustc or test binary
# of this workspace runs, since one would compete for the machine; a
# rust-analyzer flycheck starts such a build. Into <out dir> go the commit
# (rev.txt), the build log, each run's log (run-<n>.log) with its exit
# status, its wall time and the load average before it (runs.txt), and the
# per-binary table of dev/measure/test-times.py (test-times.txt). The wall
# time is cargo's whole run, doc tests included, which the binaries' sum of
# `finished in` leaves out.
set -euo pipefail
[[ $# -ge 1 ]] || { sed -n '5p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
out=$1
runs=${2:-2}
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$out"

# Stops when a build or a test of this workspace already runs.
idle() {
  local busy
  busy=$(pgrep -a -x 'cargo|rustc' || true)
  busy+=$(pgrep -a -f "$PWD/target/debug/deps/" || true)
  if [[ -n $busy ]]; then
    printf 'another build or test runs:\n%s\n' "$busy" >&2
    exit 1
  fi
}

{ git rev-parse HEAD; git status --short; } > "$out/rev.txt"
idle
cargo test --locked --workspace --no-run -j 4 > "$out/build.log" 2>&1
: > "$out/runs.txt"
args=()
for n in $(seq 1 "$runs"); do
  idle
  printf 'run-%d %s %s\n' "$n" "$(date -Is)" "$(cut -d' ' -f1-3 /proc/loadavg)" >> "$out/runs.txt"
  status=0
  start=$(date +%s%N)
  cargo test --locked --workspace -j 4 > "$out/run-$n.log" 2>&1 || status=$?
  end=$(date +%s%N)
  printf 'run-%d exit %d, wall %d ms\n' "$n" "$status" $(((end - start) / 1000000)) >> "$out/runs.txt"
  args+=("$n=$out/run-$n.log")
done
python3 dev/measure/test-times.py "${args[@]}" | tee "$out/test-times.txt"
