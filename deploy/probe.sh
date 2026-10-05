#!/usr/bin/env bash
# Runs the file probe (crates/trss-probe, ticket 0060) in a one-off container
# that looks like trss-worker's: the same image, the user 1000:1000, the same
# memory limit (256M, no swap) and CPU limit, and the worker's mounts, with the
# probe binary mounted read-only from the host. It works with a release whose
# image has no probe in it; build the binary with
#
#   docker build --target probe-binary --output type=local,dest=probe-out .
#
# and copy probe-out/trss-probe and probe-out/trss-extract (the child program
# of the worker's unpacking, for the probe's --unpack) next to this script on
# the host.
#
# Run it in the folder that holds docker-compose.trss.yml and its .env. It
# reads MEDIA_DIR, TRSS_DATA_DIR, TRSS_VERSION, TRSS_UID and TRSS_GID from the
# .env file (the same defaults as the compose file).
#
#   ./probe.sh [options] [-- probe options]
#
#   --probe FILE     the static trss-probe binary (default: trss-probe next to
#                    this script)
#   --extract FILE   the static trss-extract binary, mounted at /trss-extract
#                    where the probe looks for it (default: trss-extract next to
#                    this script; not mounted when the file does not exist)
#   --samples DIR    a host folder of archives, mounted read-only at /samples,
#                    for the probe's --unpack
#   --env FILE       the compose .env (default: ./.env)
#   --image IMAGE    the image (default: the compose file's, with TRSS_VERSION)
#   --media-dir DIR  the host folder mounted at /downloads (default: MEDIA_DIR)
#   --data-dir DIR   the host folder mounted at /data. By default a new folder
#                    next to TRSS_DATA_DIR (same disk), made for this run and
#                    removed after: the real data folder belongs to root until
#                    it is switched to 1000:1000 (ticket 0062), and the probe
#                    makes files in it as 1000:1000. Pass the real one only
#                    after that.
#   --memory SIZE    the memory limit (default: 256m, as the worker's)
#
# Everything after `--` goes to the probe, for example
#
#   ./probe.sh -- --work /downloads/downloads/<a folder Transmission made>
#   ./probe.sh -- --media /downloads/downloads --size-mib 400
#   ./probe.sh --samples ~/samples -- --size-mib 0 --unpack /samples/big.rar \
#     --unpack /samples/split.part1.rar,/samples/split.part2.rar
#
# The probe's options are in `trss-probe --help`. It makes folders named
# `.trss-probe-*` under the folders it is given and removes them again.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
probe="$here/trss-probe"
extract="$here/trss-extract"
extract_given=""
samples=""
env_file=".env"
image=""
media_dir=""
data_dir=""
memory="256m"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --probe) probe="$2"; shift 2 ;;
    --extract) extract="$2"; extract_given=1; shift 2 ;;
    --samples) samples="$2"; shift 2 ;;
    --env) env_file="$2"; shift 2 ;;
    --image) image="$2"; shift 2 ;;
    --media-dir) media_dir="$2"; shift 2 ;;
    --data-dir) data_dir="$2"; shift 2 ;;
    --memory) memory="$2"; shift 2 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d;s/^# \{0,1\}//'; exit 0 ;;
    --) shift; break ;;
    *) echo "probe.sh: unknown option $1 (probe options go after --)" >&2; exit 2 ;;
  esac
done

[[ -f "$env_file" ]] || { echo "probe.sh: no $env_file here; run it in the compose folder or pass --env" >&2; exit 2; }
[[ -f "$probe" ]] || { echo "probe.sh: no probe binary at $probe (build it, see the top of this script)" >&2; exit 2; }
chmod a+rx "$probe" 2>/dev/null || true
if [[ -f "$extract" ]]; then
  extract="$(realpath -- "$extract")"
  chmod a+rx "$extract" 2>/dev/null || true
elif [[ -n "$extract_given" ]]; then
  echo "probe.sh: no trss-extract at $extract (--extract)" >&2; exit 2
else
  extract=""
fi
if [[ -n "$samples" ]]; then
  [[ -d "$samples" ]] || { echo "probe.sh: $samples is not a folder (--samples)" >&2; exit 2; }
  samples="$(realpath -- "$samples")"
fi
env_dir="$(cd -- "$(dirname -- "$env_file")" && pwd)"

# KEY=value from the .env file: the last one, without a trailing comment and
# quotes.
env_value() {
  local line
  line="$(grep -E "^$1=" "$env_file" | tail -n 1 || true)"
  [[ -n "$line" ]] || return 0
  line="${line#*=}"
  line="$(sed -E 's/[[:space:]]+#.*$//; s/^"(.*)"$/\1/; s/^'"'"'(.*)'"'"'$/\1/' <<<"$line")"
  printf '%s' "$line"
}

# A path from the .env file: relative to the folder of the file, as compose does.
host_path() {
  (cd -- "$env_dir" && realpath -m -- "$1")
}

version="$(env_value TRSS_VERSION)"
uid="$(env_value TRSS_UID)"; uid="${uid:-1000}"
gid="$(env_value TRSS_GID)"; gid="${gid:-1000}"
[[ -n "$image" ]] || image="ghcr.io/syrflover/transmission-rss:${version:?TRSS_VERSION is not in $env_file; pass --image}"
[[ -n "$media_dir" ]] || media_dir="$(env_value MEDIA_DIR)"
[[ -n "$media_dir" ]] || { echo "probe.sh: MEDIA_DIR is not in $env_file; pass --media-dir" >&2; exit 2; }
media_dir="$(host_path "$media_dir")"
app_data="$(env_value TRSS_DATA_DIR)"
app_data="$(host_path "${app_data:-./data}")"

made_data=""
cleanup() {
  [[ -n "$made_data" ]] && rm -rf -- "$made_data" 2>/dev/null || true
}
trap cleanup EXIT
if [[ -z "$data_dir" ]]; then
  data_dir="$(mktemp -d "$(dirname -- "$app_data")/.trss-probe-data.XXXXXX")"
  made_data="$data_dir"
  chmod 1777 "$data_dir"
else
  data_dir="$(host_path "$data_dir")"
fi

echo "== Host"
echo "date:    $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "kernel:  $(uname -r)"
echo "os:      $(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}")"
echo "docker:  $(docker version --format '{{.Server.Version}}' 2>/dev/null || echo unknown)"
echo "image:   $image"
echo "media:   $media_dir"
echo "data:    $data_dir (app data folder: $app_data)"
echo "extract: ${extract:-none (trss-extract is not next to this script; --unpack needs it)}"
[[ -z "$samples" ]] || echo "samples: $samples (at /samples)"
echo "user:    $uid:$gid, memory $memory (swap the same), 0.25 CPU"
if command -v findmnt >/dev/null; then
  echo "host mounts:"
  findmnt -T "$media_dir" -o TARGET,SOURCE,FSTYPE,OPTIONS | sed 's/^/  /'
  findmnt -T "$data_dir" -o TARGET,SOURCE,FSTYPE,OPTIONS | sed -n '2p' | sed 's/^/  /'
fi
echo

mounts=()
[[ -z "$extract" ]] || mounts+=(-v "$extract":/trss-extract:ro)
[[ -z "$samples" ]] || mounts+=(-v "$samples":/samples:ro)

docker run --rm \
  --user "$uid:$gid" \
  --memory "$memory" --memory-swap "$memory" --cpus 0.25 \
  --network none \
  -v "$media_dir":/downloads \
  -v "$data_dir":/data \
  -v "$probe":/trss-probe:ro \
  ${mounts[@]+"${mounts[@]}"} \
  --entrypoint /trss-probe \
  "$image" "$@"
