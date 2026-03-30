#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

COMPOSE_FILE="$PROJECT_DIR/docker-compose.trss.yml"
ENV_FILE="$PROJECT_DIR/.env"
LOG_FILE="$PROJECT_DIR/logs/trss-cron.log"

MARKER_BEGIN="# BEGIN trss-cron"
MARKER_END="# END trss-cron"

CRON_SCHEDULE="*/5 * * * *"

cmd_run() {
    echo "--- $(date '+%Y-%m-%d %H:%M:%S') ---"
    docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" -p trss-cron run --rm trss
}

cmd_install() {
    mkdir -p "$(dirname "$LOG_FILE")"

    CRON_ENTRY="${MARKER_BEGIN}
${CRON_SCHEDULE} ${SCRIPT_DIR}/cron.sh run >> ${LOG_FILE} 2>&1
${MARKER_END}"

    EXISTING=$(crontab -l 2>/dev/null || true)
    CLEANED=$(echo "$EXISTING" | sed "/${MARKER_BEGIN}/,/${MARKER_END}/d" | sed '/^[[:space:]]*$/d')

    if [ -n "$CLEANED" ]; then
        printf '%s\n%s\n' "$CLEANED" "$CRON_ENTRY" | crontab -
    else
        echo "$CRON_ENTRY" | crontab -
    fi

    echo "Installed trss cron (${CRON_SCHEDULE})"
    echo "Log: ${LOG_FILE}"
}

cmd_uninstall() {
    EXISTING=$(crontab -l 2>/dev/null || true)

    if echo "$EXISTING" | grep -q "$MARKER_BEGIN"; then
        echo "$EXISTING" | sed "/${MARKER_BEGIN}/,/${MARKER_END}/d" | crontab -
        echo "Uninstalled trss cron"
    else
        echo "No trss cron found"
    fi
}

case "${1:-}" in
    run)       cmd_run ;;
    install)   cmd_install ;;
    uninstall) cmd_uninstall ;;
    *)
        echo "Usage: $0 {install|uninstall|run}"
        exit 1
        ;;
esac
