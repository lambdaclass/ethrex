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
  local method=$1 path=$2 retry=()
  # Only retry reads: a retried start/stop that got a 4xx ("already
  # starting") would just fail again.
  [[ $method == GET ]] && retry=(--retry 3 --retry-all-errors)
  # The key goes through a config file descriptor, not argv, so it does not
  # show up in `ps`.
  curl --silent --show-error --fail --max-time 30 ${retry[@]+"${retry[@]}"} \
    -X "$method" \
    --config <(printf 'header = "Authorization: Bearer %s"\n' "$PAPERSPACE_API_KEY") \
    -H "Content-Type: application/json" \
    "${API}${path}"
}

# Prints the machine state in lower case. The API does not document the
# possible values; the console calls them "Off", "Starting up", "On/Ready",
# "Shutting Down", ... and the API example shows "off".
state() {
  api GET "/machines/${PAPERSPACE_MACHINE_ID}" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["state"].lower())' 2>/dev/null
}

is_ready() { [[ $1 == ready || $1 == on || $1 == running ]]; }

case "${1:-}" in
  state)
    state
    ;;
  start)
    deadline=$((SECONDS + START_TIMEOUT_SECS))
    off_polls=0 # polls that saw "off" since the last start request
    ready_polls=0
    while ((SECONDS < deadline)); do
      if ! s=$(state); then
        echo "state query failed, retrying" >&2
        sleep 15
        continue
      fi
      echo "machine ${PAPERSPACE_MACHINE_ID}: ${s}"
      if is_ready "$s"; then
        # Require two ready polls 30s apart: a stop the idle watchdog sent
        # just before this run was created may not show up in the state yet.
        ((++ready_polls >= 2)) && exit 0
        sleep 30
        continue
      fi
      ready_polls=0
      if [[ $s == off ]]; then
        # Send start, and send it again if the machine still reports off
        # a minute later (4 polls).
        if ((off_polls % 4 == 0)); then
          echo "starting machine ${PAPERSPACE_MACHINE_ID}"
          api PATCH "/machines/${PAPERSPACE_MACHINE_ID}/start" >/dev/null ||
            echo "start request failed, retrying" >&2
        fi
        off_polls=$((off_polls + 1))
      else
        off_polls=0
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
