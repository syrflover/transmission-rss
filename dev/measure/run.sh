#!/usr/bin/env bash
# A load measurement of an app image in the development environment
# (dev/compose.sh), under the compose file's limits as in production: trss
# 0.5 CPU and 384M (tickets 0116, 0117 and 0146; until 0146 the web and the
# worker were two containers, measured each under its own limits).
#
#   dev/measure/run.sh snapshot           keep dev/local/data as dev/local/measure-snapshot
#   dev/measure/run.sh <image> <out dir>  one measurement of <image>
#
# A measurement:
# - puts dev/local/data back to the snapshot and runs <image> as trss, so
#   every measurement starts from the same data;
# - waits for the first cycle, then sends load.py's web load (two rounds to
#   warm up, then 30) and 30 rescans of every watch folder;
# - writes into <out dir> the container's cgroup counters after each phase
#   (stats.txt: cpu.stat, memory.current, memory.peak, memory.events, page
#   faults, threads), the loads' results, the log, and `perf record -e
#   cpu-clock:u` of the process (trss.perf).
#
# <image> is tagged as the environment's image, which stays running on the
# snapshot's data afterwards; `dev/compose.sh up` builds the checkout's again.
# perf samples only user space unless perf_event_paranoid is 1 or less; to
# read the symbols, copy the image's /usr/local/bin (and a dynamic libc with
# its debug file) under a folder and pass it to `perf report --symfs`.
set -euo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$REPO/dev/measure"
SNAPSHOT="$REPO/dev/local/measure-snapshot"
IMAGE=ghcr.io/syrflover/transmission-rss:local
cd "$REPO"

trss() { docker compose --project-directory "$REPO" --env-file "$REPO/dev/dev.env" -f docker-compose.trss.yml -f dev/trss.dev.yml "$@"; }

if [[ "${1:-}" == snapshot ]]; then
  trss stop trss
  rm -rf "$SNAPSHOT"
  cp -a dev/local/data "$SNAPSHOT"
  rm -f "$SNAPSHOT"/*.lock
  trss start trss
  exit 0
fi
[[ $# -eq 2 ]] || { sed -n '6,7p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
[[ -d "$SNAPSHOT" ]] || { echo "no $SNAPSHOT: run '$0 snapshot' first" >&2; exit 1; }
image=$1
OUT="$(mkdir -p "$2" && cd "$2" && pwd)"
rm -f "$OUT"/*

now() { date +%s.%N; }
since() { awk -v a="$(now)" -v b="$1" 'BEGIN{printf "%.2f", a-b}'; }
cgroup() { echo "/sys/fs/cgroup/system.slice/docker-$(docker inspect -f '{{.Id}}' "$1").scope"; }
pid() { docker inspect -f '{{.State.Pid}}' "$1"; }
value() { awk -v k="$1" '$1==k{print $2}' "$2"; }
stats() {
  local c d
  local c=trss d
  d="$(cgroup "$c")"
  echo "$1 $c t=$(now) $(awk '{printf "%s=%s ", $1, $2}' "$d/cpu.stat")mem_current=$(cat "$d/memory.current") mem_peak=$(cat "$d/memory.peak") oom_kill=$(value oom_kill "$d/memory.events") ev_high=$(value high "$d/memory.events") ev_max=$(value max "$d/memory.events") pgfault=$(value pgfault "$d/memory.stat") pgmajfault=$(value pgmajfault "$d/memory.stat") threads=$(ls /proc/"$(pid "$c")"/task | wc -l)" >>"$OUT/stats.txt"
}

trss stop trss >/dev/null 2>&1
rsync -a --delete "$SNAPSHOT/" dev/local/data/
docker tag "$image" "$IMAGE"
start=$(now)
trss up -d --force-recreate --no-deps trss >/dev/null 2>&1
until curl -sf -o /dev/null 127.0.0.1:8080/api/health; do sleep 0.2; done
echo "web_ready_s $(since "$start")" >>"$OUT/times.txt"
until docker logs trss 2>&1 | grep -q '^Cycle finished'; do sleep 0.2; done
echo "first_cycle_s $(since "$start")" >>"$OUT/times.txt"
stats start

python3 "$HERE/load.py" web 2 >"$OUT/warmup.json"
stats warm

perf record -q -e cpu-clock:u -F 1999 -g -p "$(pid trss)" -o "$OUT/trss.perf" 2>"$OUT/perf-trss.log" &
perf_trss=$!
sleep 1

python3 "$HERE/load.py" web 30 >"$OUT/web.json"
stats web
for _ in $(seq 30); do python3 "$HERE/load.py" rescan; done >"$OUT/rescan.json"
stats rescan

kill -INT "$perf_trss"
wait "$perf_trss" || true
grep -E '^(anon|file|kernel|sock|shmem) ' "$(cgroup trss)/memory.stat" | sed "s/^/trss /" >>"$OUT/memory-stat.txt"
docker logs trss >"$OUT/trss.log" 2>&1
echo "$image: $OUT"
