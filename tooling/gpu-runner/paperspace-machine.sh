#!/usr/bin/env bash
# Start, stop or query a Paperspace machine through the Paperspace API.
#
# Usage: paperspace-machine.sh <state|start|stop>
#
# Environment:
#   PAPERSPACE_API_KEY      API key (Paperspace console -> Team settings -> API keys)
#   PAPERSPACE_MACHINE_ID   machine ID, e.g. pszj5x2xbgeb
#   START_TIMEOUT_SECS      how long `start` waits for the machine to be ready (default 600)
#
# `start` is idempotent: it does nothing if the machine is already on, waits
# out a shutdown in progress, and only returns once the machine is ready.
#
# API reference: https://docs.digitalocean.com/reference/paperspace/api-reference/machine/
set -euo pipefail

API="https://api.paperspace.com/v1"
: "${PAPERSPACE_API_KEY:?PAPERSPACE_API_KEY is not set}"
: "${PAPERSPACE_MACHINE_ID:?PAPERSPACE_MACHINE_ID is not set}"
START_TIMEOUT_SECS="${START_TIMEOUT_SECS:-600}"

api() {
  local method=$1 path=$2
  curl --silent --show-error --fail --max-time 30 --retry 3 --retry-all-errors \
    -X "$method" \
    -H "Authorization: Bearer ${PAPERSPACE_API_KEY}" \
    -H "Content-Type: application/json" \
    "${API}${path}"
}

# Prints the machine state in lower case. The API does not document the
# possible values; the console calls them "Off", "Starting up", "On/Ready",
# "Shutting Down", ... and the API example shows "off".
state() {
  api GET "/machines/${PAPERSPACE_MACHINE_ID}" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["state"].lower())'
}

is_ready() { [[ $1 == ready || $1 == on || $1 == running ]]; }

case "${1:-}" in
  state)
    state
    ;;
  start)
    deadline=$((SECONDS + START_TIMEOUT_SECS))
    start_sent=false
    while ((SECONDS < deadline)); do
      s=$(state)
      echo "machine ${PAPERSPACE_MACHINE_ID}: ${s}"
      if is_ready "$s"; then
        exit 0
      fi
      if [[ $s == off && $start_sent == false ]]; then
        echo "starting machine ${PAPERSPACE_MACHINE_ID}"
        if api PATCH "/machines/${PAPERSPACE_MACHINE_ID}/start" >/dev/null; then
          start_sent=true
        fi
      elif [[ $s == off ]]; then
        # Start was accepted but the machine is still reported off: retry it.
        start_sent=false
      fi
      sleep 15
    done
    echo "timed out after ${START_TIMEOUT_SECS}s waiting for machine ${PAPERSPACE_MACHINE_ID}" >&2
    exit 1
    ;;
  stop)
    echo "stopping machine ${PAPERSPACE_MACHINE_ID}"
    api PATCH "/machines/${PAPERSPACE_MACHINE_ID}/stop" >/dev/null
    ;;
  *)
    echo "usage: $0 <state|start|stop>" >&2
    exit 2
    ;;
esac
