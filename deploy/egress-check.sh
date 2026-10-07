#!/usr/bin/env bash
# Asks the server browser's egress proxy (crates/trss-browser/src/launcher/
# egress.rs), from inside the running trss-browser, to reach this host's LAN
# address, the gateways of the Docker networks and the container's loopback,
# the way Chromium's connections reach it, and one public site as a control
# (ticket 0082). The proxy must refuse every local destination with 403 and
# let the public one through with 200. The image has no curl, so the requests
# are written with bash's /dev/tcp; the proxy listens on a port of the
# container's loopback chosen when the launcher starts, found from
# /proc/net/tcp by which port answers the control as a proxy.
#
# Run it in the folder that holds docker-compose.trss.yml and its .env. It
# reads TRSS_WEB_HOST_IP and TRSS_WEB_HOST_PORT from the .env file. Nothing is
# changed; the control opens one connection to example.com:443 and closes it.
#
#   ./egress-check.sh [--env FILE] [--container NAME]
set -euo pipefail

env_file=".env"
container="trss-browser"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --env) env_file="$2"; shift 2 ;;
    --container) container="$2"; shift 2 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d;s/^# \{0,1\}//'; exit 0 ;;
    *) echo "egress-check.sh: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done
[[ -f "$env_file" ]] || { echo "egress-check.sh: no $env_file here; run it in the compose folder or pass --env" >&2; exit 2; }

# KEY=value from the .env file: the last one, without a trailing comment and
# quotes (as probe.sh reads it).
env_value() {
  local line
  line="$(grep -E "^$1=" "$env_file" | tail -n 1 || true)"
  [[ -n "$line" ]] || return 0
  line="${line#*=}"
  line="$(sed -E 's/[[:space:]]+#.*$//; s/^"(.*)"$/\1/; s/^'"'"'(.*)'"'"'$/\1/' <<<"$line")"
  printf '%s' "$line"
}

lan="$(env_value TRSS_WEB_HOST_IP)"
[[ -n "$lan" ]] || { echo "egress-check.sh: TRSS_WEB_HOST_IP is not in $env_file" >&2; exit 2; }
web_port="$(env_value TRSS_WEB_HOST_PORT)"; web_port="${web_port:-8080}"

# The gateways of the container's own networks, of Docker's default bridge
# and of trss_net (Transmission's), each once.
gateways="$(
  {
    docker inspect -f '{{range .NetworkSettings.Networks}}{{.Gateway}}{{"\n"}}{{end}}' "$container"
    docker network inspect -f '{{range .IPAM.Config}}{{.Gateway}}{{"\n"}}{{end}}' bridge trss_net 2>/dev/null || true
  } | grep -v '^$' | sort -u
)"

targets=("$lan:$web_port" "$lan:9091")
for gateway in $gateways; do
  targets+=("$gateway:$web_port" "$gateway:9091")
done
targets+=("127.0.0.1:9230" "localhost:9230")

echo "date:      $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "container: $container ($(docker inspect -f '{{.Config.Image}}' "$container"))"
echo "LAN:       $lan (web port $web_port)"
echo "gateways:  $(tr '\n' ' ' <<<"$gateways")"
echo

docker exec -i "$container" bash -s -- "$lan:$web_port" "${targets[@]}" <<'PROBE'
set -u
web="$1"; shift

# The first line of the answer to one request ($2, a full request head) sent
# to 127.0.0.1:$1, or why there was none.
ask() {
  local line
  exec 3<>"/dev/tcp/127.0.0.1/$1" 2>/dev/null || { echo "(no connection)"; return; }
  printf '%b' "$2" >&3
  if IFS= read -r -t 20 line <&3; then echo "${line%$'\r'}"; else echo "(no answer in 20 s)"; fi
  exec 3>&- 3<&-
}

connect() { ask "$1" "CONNECT $2 HTTP/1.1\r\nHost: $2\r\n\r\n"; }

proxy=""
while read -r _ local _ state _; do
  [[ "$state" == 0A && "$local" == 0100007F:* ]] || continue
  port=$((16#${local#*:}))
  answer="$(connect "$port" example.com:443)"
  if [[ "$answer" == "HTTP/1.1 200"* ]]; then
    proxy="$port"
    echo "proxy:     127.0.0.1:$port"
    echo "control:   CONNECT example.com:443 -> $answer"
    break
  fi
done < <(tail -n +2 /proc/net/tcp)
[[ -n "$proxy" ]] || { echo "no loopback port answered CONNECT example.com:443 with 200: the proxy was not found" >&2; exit 1; }

refused=0; total=0
check() {
  local what="$1" answer="$2"
  total=$((total + 1))
  [[ "$answer" == "HTTP/1.1 403"* ]] && refused=$((refused + 1))
  printf '%-40s -> %s\n' "$what" "$answer"
}
echo
for target in "$@"; do
  check "CONNECT $target" "$(connect "$proxy" "$target")"
done
check "GET http://$web/" "$(ask "$proxy" "GET http://$web/ HTTP/1.1\r\nHost: $web\r\nConnection: close\r\n\r\n")"
echo
echo "refused with 403: $refused of $total"
[[ "$refused" == "$total" ]]
PROBE
