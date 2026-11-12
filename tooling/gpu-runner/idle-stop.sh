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
#   DRY_RUN          1 (default) only logs "would stop"; 0 really stops
set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-lambdaclass/ethrex}"
GPU_WORKFLOWS="${GPU_WORKFLOWS:-main_prover.yaml}"
IDLE_MINUTES="${IDLE_MINUTES:-15}"
DRY_RUN="${DRY_RUN:-1}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is not set}"

# /run is cleared on boot, so a fresh boot always gets a full idle window.
STATE_DIR="${STATE_DIR:-/run/gpu-runner-idle}"
LAST_BUSY="$STATE_DIR/last_busy"
mkdir -p "$STATE_DIR"
now=$(date +%s)
[[ -f $LAST_BUSY ]] || echo "$now" >"$LAST_BUSY"

# Number of queued and in-progress runs of the GPU workflows. Prints nothing
# if the GitHub API cannot be reached.
github_active_runs() {
  local total=0 wf status n
  for wf in $GPU_WORKFLOWS; do
    for status in queued in_progress; do
      n=$(curl --silent --show-error --fail --max-time 30 \
        -H "Authorization: Bearer ${GITHUB_TOKEN}" \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${GITHUB_REPO}/actions/workflows/${wf}/runs?status=${status}&per_page=1" |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["total_count"])' 2>/dev/null) || return 0
      total=$((total + n))
    done
  done
  echo "$total"
}

if pgrep -f Runner.Worker >/dev/null; then
  echo "$now" >"$LAST_BUSY"
  echo "busy: job running"
  exit 0
fi

runs=$(github_active_runs)
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

idle_secs=$((now - $(cat "$LAST_BUSY")))
if ((idle_secs < IDLE_MINUTES * 60)); then
  echo "idle for $((idle_secs / 60))m (stop at ${IDLE_MINUTES}m)"
  exit 0
fi

if [[ $DRY_RUN == 1 ]]; then
  echo "idle for $((idle_secs / 60))m: would stop (DRY_RUN=1)"
  exit 0
fi

echo "idle for $((idle_secs / 60))m: stopping"
"$(dirname "$(readlink -f "$0")")/paperspace-machine.sh" stop
