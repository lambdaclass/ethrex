#!/usr/bin/env bash
# Stop this Paperspace machine once the GitHub Actions runner has been idle
# for IDLE_MINUTES. Run every minute by gpu-runner-idle.timer.
#
# The machine counts as busy while:
#   - a job is running on it (a Runner.Worker process exists), or
#   - a run of a workflow that needs the GPU runner is queued or in progress
#     on GitHub (so a run that just started the machine is not cut off before
#     the runner picks up its job).
#
# Environment (from /etc/gpu-runner-idle.env):
#   PAPERSPACE_API_KEY, PAPERSPACE_MACHINE_ID   see paperspace-machine.sh
#   GITHUB_TOKEN     fine-grained token, Actions: read on GITHUB_REPO
#   GITHUB_REPO      default lambdaclass/ethrex
#   GPU_WORKFLOWS    space-separated workflow files using the GPU runner
#                    (default main_prover.yaml)
#   IDLE_MINUTES     default 15
#   DRY_RUN          exactly 0 really stops; anything else (default 1) only
#                    logs "would stop"
set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-lambdaclass/ethrex}"
GPU_WORKFLOWS="${GPU_WORKFLOWS:-main_prover.yaml}"
IDLE_MINUTES="${IDLE_MINUTES:-15}"
DRY_RUN="${DRY_RUN:-1}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is not set}"
[[ $IDLE_MINUTES =~ ^[0-9]+$ ]] || {
  echo "IDLE_MINUTES must be a whole number of minutes, got '${IDLE_MINUTES}'" >&2
  exit 1
}

# /run is cleared on boot, so a fresh boot always gets a full idle window.
STATE_DIR="${STATE_DIR:-/run/gpu-runner-idle}"
LAST_BUSY="$STATE_DIR/last_busy"
mkdir -p "$STATE_DIR"
now=$(date +%s)
# Reset a missing or corrupt timestamp (e.g. a truncated write) to now.
last_busy=$(cat "$LAST_BUSY" 2>/dev/null || true)
if ! [[ $last_busy =~ ^[0-9]+$ ]]; then
  last_busy=$now
  echo "$now" >"$LAST_BUSY"
fi

# Number of queued and in-progress runs of the GPU workflows. Prints "auth"
# if GitHub rejects the token, and nothing if the API cannot be reached.
github_active_runs() {
  local total=0 wf status out code n
  for wf in $GPU_WORKFLOWS; do
    for status in queued in_progress; do
      # The token goes through a config file descriptor, not argv, so it
      # does not show up in `ps`.
      out=$(curl --silent --show-error --max-time 30 \
        --config <(printf 'header = "Authorization: Bearer %s"\n' "$GITHUB_TOKEN") \
        -H "Accept: application/vnd.github+json" \
        -w '\n%{http_code}' \
        "https://api.github.com/repos/${GITHUB_REPO}/actions/workflows/${wf}/runs?status=${status}&per_page=1") ||
        return 0
      code=${out##*$'\n'}
      if [[ $code == 401 || $code == 403 ]]; then
        echo auth
        return 0
      fi
      [[ $code == 200 ]] || return 0
      n=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["total_count"])' \
        <<<"${out%$'\n'*}" 2>/dev/null) || return 0
      total=$((total + n))
    done
  done
  echo "$total"
}

# Match the process name exactly: `-f` would also match any shell or pager
# whose command line mentions Runner.Worker.
if pgrep -x Runner.Worker >/dev/null; then
  echo "$now" >"$LAST_BUSY"
  echo "busy: job running"
  exit 0
fi

runs=$(github_active_runs)
if [[ $runs == auth ]]; then
  # An expired or revoked token would otherwise keep the machine on forever
  # without anyone noticing: fail the unit so `systemctl --failed` shows it.
  echo "GitHub rejected GITHUB_TOKEN (HTTP 401/403): renew it in /etc/gpu-runner-idle.env; not stopping" >&2
  exit 1
fi
if [[ -z $runs ]]; then
  # Can't tell whether a run is waiting: stay on rather than cut it off.
  # Paperspace auto-shutdown is the backstop if this keeps failing.
  echo "busy: GitHub API unreachable, not stopping" >&2
  exit 0
fi
if ((runs > 0)); then
  echo "$now" >"$LAST_BUSY"
  echo "busy: ${runs} GPU workflow run(s) queued or in progress"
  exit 0
fi

idle_secs=$((now - last_busy))
if ((idle_secs < IDLE_MINUTES * 60)); then
  echo "idle for $((idle_secs / 60))m (stop at ${IDLE_MINUTES}m)"
  exit 0
fi

# Fail closed: only an exact 0 stops the machine, so a typo or a trailing
# comment in the env file keeps it in dry-run mode.
if [[ $DRY_RUN != 0 ]]; then
  echo "idle for $((idle_secs / 60))m: would stop (DRY_RUN=${DRY_RUN})"
  exit 0
fi

echo "idle for $((idle_secs / 60))m: stopping"
"$(dirname "$(readlink -f "$0")")/paperspace-machine.sh" stop
