#!/usr/bin/env bash
# Start, stop or query a Paperspace machine through the Paperspace API.
#
# Usage: paperspace-machine.sh <state|start|stop>
#
# CI uses `start` (.github/actions/start-paperspace-machine). `state` and
# `stop` are for manual use: the idle watchdog powers the machine off from
# inside instead, so the API key is never stored on it.
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

# Prints the response body followed by a line with the HTTP status code.
# Fails only if the request itself fails (DNS, connection, timeout).
api() {
  local method=$1 path=$2 retry=()
  # Only retry reads: a retried start/stop that got a 4xx ("already
  # starting") would just fail again. Without --fail, --retry still retries
  # connection errors and transient HTTP codes (408, 429, 5xx).
  [[ $method == GET ]] && retry=(--retry 3 --retry-connrefused)
  # The key goes through a config file descriptor, not argv, so it does not
  # show up in `ps`.
  # No Content-Type header: none of these requests has a body, and the API
  # answers PATCH .../start with 400 when it gets `Content-Type:
  # application/json` and an empty body (even though its docs show that
  # header). Without it, start returns 200.
  curl --silent --show-error --max-time 30 ${retry[@]+"${retry[@]}"} \
    -X "$method" \
    --config <(printf 'header = "Authorization: Bearer %s"\n' "$PAPERSPACE_API_KEY") \
    -w '\n%{http_code}' \
    "${API}${path}"
}

# Returns 2 for an HTTP status that retrying won't fix: bad API key (401/403)
# or wrong machine ID (404).
check_code() {
  local code=$1
  case $code in
    2??) return 0 ;;
    401 | 403 | 404)
      echo "Paperspace API refused the request (HTTP ${code}): check PAPERSPACE_API_KEY and PAPERSPACE_MACHINE_ID" >&2
      return 2
      ;;
    *)
      echo "Paperspace API returned HTTP ${code}" >&2
      return 1
      ;;
  esac
}

# Prints the machine state in lower case. The API does not document the
# possible values; on the real machine we saw "off", "starting", "ready" and
# "stopping". Returns 2 for a permanent error (see check_code), 1 otherwise.
state() {
  local out
  out=$(api GET "/machines/${PAPERSPACE_MACHINE_ID}") || return 1
  check_code "${out##*$'\n'}" || return
  python3 -c 'import json,sys; print(json.load(sys.stdin)["state"].lower())' \
    <<<"${out%$'\n'*}" 2>/dev/null
}

# Sends start/stop. Returns like state().
action() {
  local out
  out=$(api PATCH "/machines/${PAPERSPACE_MACHINE_ID}/$1") || return 1
  check_code "${out##*$'\n'}"
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
      rc=0
      s=$(state) || rc=$?
      # A bad key or machine ID won't fix itself: fail now instead of after
      # START_TIMEOUT_SECS.
      ((rc == 2)) && exit 1
      if ((rc != 0)); then
        echo "state query failed, retrying" >&2
        sleep 15
        continue
      fi
      echo "machine ${PAPERSPACE_MACHINE_ID}: ${s}"
      if is_ready "$s"; then
        # Require the machine to stay ready for 30s (2 polls, 30s apart): the
        # idle watchdog may have powered it off just before this run was
        # created. Measured on the real machine, the API reports "stopping"
        # within ~2s of `systemctl poweroff` and "off" ~14s later, so 30s
        # leaves a wide margin.
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
          rc=0
          action start || rc=$?
          ((rc == 2)) && exit 1
          ((rc == 0)) || echo "start request failed, retrying" >&2
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
    action stop
    ;;
  *)
    echo "usage: $0 <state|start|stop>" >&2
    exit 2
    ;;
esac
