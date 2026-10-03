#!/usr/bin/env bash
# The local development environment: the repository's compose files run here
# with a locally built image, a local Transmission that keeps added torrents
# stopped, and a copy of a server's data.
#
#   dev/compose.sh pull [--force]   copy the server's database, covers and media file names
#   dev/compose.sh up [--no-build]  build the images and start Transmission, trss-browser, trss-worker, trss-web
#   dev/compose.sh down             stop and remove the containers (dev/local stays)
#   dev/compose.sh logs [service]   follow the logs (default: trss-worker)
#   dev/compose.sh status           show the containers
#
# The worker also reads dev/trss.dev.yml, which turns on the fake subtitle
# source (posts on fake.trss.invalid) for subtitle jobs.
#
# `pull` reaches the server over ssh: TRSS_DEV_SERVER (default j4105) and
# TRSS_DEV_SERVER_DIR, the folder of its compose files relative to the remote
# home (default trss).
set -euo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DEV="$REPO/dev"
LOCAL="$DEV/local"
IMAGE=ghcr.io/syrflover/transmission-rss:local
# The server browser (Dockerfile.browser). Always up in compose, with only a
# virtual display and its launcher running while no job uses a browser.
BROWSER_IMAGE=ghcr.io/syrflover/trss-browser:local
SERVER="${TRSS_DEV_SERVER:-j4105}"
SERVER_DIR="${TRSS_DEV_SERVER_DIR:-trss}"

compose() {
  docker compose --project-directory "$REPO" --env-file "$DEV/dev.env" "$@"
}

# docker-compose.trss.yml with the development additions.
trss() {
  compose -f docker-compose.trss.yml -f dev/trss.dev.yml "$@"
}

prepare() {
  mkdir -p "$LOCAL/data" "$LOCAL/media/downloads" "$LOCAL/watch" "$LOCAL/transmission-config" "$LOCAL/data/browser-downloads"
  # Transmission writes its settings back on start; seed them only once.
  [[ -f "$LOCAL/transmission-config/settings.json" ]] ||
    cp "$DEV/transmission-settings.json" "$LOCAL/transmission-config/settings.json"
}

# Runs on the server. Writes a tar of the app data folder and the media file
# names to stdout; everything else goes to stderr.
REMOTE_SCRIPT='
set -euo pipefail
cd "$SERVER_DIR"
tmp="$(mktemp -d)"
restart() { docker compose -f docker-compose.trss.yml start trss-worker trss-web >&2; }
trap "restart; rm -rf \"\$tmp\"" EXIT
# Stopped, so the database and its WAL are one consistent state.
docker compose -f docker-compose.trss.yml stop trss-worker trss-web >&2
docker cp trss-worker:/data "$tmp/data" >&2
restart
trap "rm -rf \"\$tmp\"" EXIT
rm -f "$tmp"/data/*.lock
docker exec trss-worker find /downloads -mindepth 1 -type d >"$tmp/dirs.txt"
docker exec trss-worker find /downloads -mindepth 1 -type f >"$tmp/files.txt"
tar -C "$tmp" -cf - data dirs.txt files.txt
'

# The server's paths under /downloads, as paths under dev/local/media.
local_paths() {
  { grep -a '^/downloads/' "$1" || true; } | sed "s#^/downloads/#$LOCAL/media/#"
}

pull() {
  local force=0
  [[ "${1:-}" == "--force" ]] && force=1
  if [[ -e "$LOCAL/data/trss.db" && $force -eq 0 ]]; then
    echo "dev/local already holds a database; pass --force to replace it and the media replica" >&2
    exit 1
  fi
  trss down 2>/dev/null || true

  mkdir -p "$LOCAL"
  staging="$(mktemp -d "$LOCAL/.pull.XXXXXX")"
  trap 'rm -rf "$staging"' EXIT
  echo "Copying from $SERVER:~/$SERVER_DIR (trss-worker and trss-web stop for the copy)..." >&2
  ssh "$SERVER" "SERVER_DIR='$SERVER_DIR' bash -s" <<<"$REMOTE_SCRIPT" | tar -x -C "$staging"
  [[ -f "$staging/data/trss.db" ]] || { echo "the server sent no trss.db" >&2; exit 1; }

  rm -rf "$LOCAL/data" "$LOCAL/media"
  prepare
  rm -rf "$LOCAL/data"
  mv "$staging/data" "$LOCAL/data"
  # Empty files under the server's names: reading a watch folder looks at names
  # and folder metadata only, so the library reads as it does on the server.
  local media="$LOCAL/media"
  local_paths "$staging/dirs.txt" | xargs -r -d '\n' mkdir -p
  local_paths "$staging/files.txt" | xargs -r -d '\n' dirname | sort -u | xargs -r -d '\n' mkdir -p
  local_paths "$staging/files.txt" | xargs -r -d '\n' touch
  mkdir -p "$media/downloads"
  echo "Copied: $(wc -l <"$staging/dirs.txt") folders, $(wc -l <"$staging/files.txt") files, $(find "$LOCAL/data/artwork" -type f 2>/dev/null | wc -l) covers" >&2
}

up() {
  prepare
  [[ -f "$LOCAL/data/trss.db" ]] || echo "No database in dev/local/data: trss starts empty (dev/compose.sh pull copies the server's)" >&2
  if [[ "${1:-}" != "--no-build" ]]; then
    docker build -t "$IMAGE" "$REPO"
    docker build -t "$BROWSER_IMAGE" -f "$REPO/Dockerfile.browser" "$REPO"
  fi
  compose -f docker-compose.yml up -d
  trss up -d
  echo "Web: http://localhost:8080  Transmission: http://localhost:9091" >&2
}

down() {
  trss down
  compose -f docker-compose.yml down
}

case "${1:-}" in
  pull) shift; pull "$@" ;;
  up) shift; up "$@" ;;
  down) down ;;
  logs) docker logs -f "${2:-trss-worker}" ;;
  status) compose -f docker-compose.yml ps; trss ps ;;
  *) sed -n '6,10p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
