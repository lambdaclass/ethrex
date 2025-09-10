#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_job_failure.sh <slack_webhook_url>
# Expects the following env vars (provided by the caller workflow):
#   REPO, WORKFLOW_NAME, CONCLUSION, RUN_HTML_URL, RUN_ID, HEAD_SHA
# Optional for per-job alerts:
#   JOB_NAME, JOB_HTML_URL
#   JOB_STARTED_AT, JOB_COMPLETED_AT, TRIGGER_EVENT

SLACK_WEBHOOK_URL=${1:-}
if [[ -z "${SLACK_WEBHOOK_URL}" ]]; then
  echo "Slack webhook URL not provided; skipping notification." >&2
  exit 0
fi

REPO=${REPO:-}
WORKFLOW_NAME=${WORKFLOW_NAME:-}
CONCLUSION=${CONCLUSION:-}
RUN_HTML_URL=${RUN_HTML_URL:-}
RUN_ID=${RUN_ID:-}
HEAD_SHA=${HEAD_SHA:-}
JOB_NAME=${JOB_NAME:-}
JOB_HTML_URL=${JOB_HTML_URL:-}
JOB_STARTED_AT=${JOB_STARTED_AT:-}
JOB_COMPLETED_AT=${JOB_COMPLETED_AT:-}
TRIGGER_EVENT=${TRIGGER_EVENT:-}

# Ensure required tools exist (present on GitHub-hosted runners)
if ! command -v jq >/dev/null 2>&1; then
  echo "jq not found; skipping Slack notification." >&2
  exit 0
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl not found; skipping Slack notification." >&2
  exit 0
fi

RUN_URL="$RUN_HTML_URL"
if [[ -z "$RUN_URL" ]]; then
  RUN_URL="https://github.com/${REPO}/actions/runs/${RUN_ID}"
fi

SHORT_SHA="${HEAD_SHA:0:8}"
COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"

# Compute job duration if timestamps are available (GNU date on ubuntu-latest)
DURATION=""
if [[ -n "$JOB_STARTED_AT" && -n "$JOB_COMPLETED_AT" ]]; then
  if START_EPOCH=$(date -d "$JOB_STARTED_AT" +%s 2>/dev/null) && END_EPOCH=$(date -d "$JOB_COMPLETED_AT" +%s 2>/dev/null); then
    SECS=$(( END_EPOCH - START_EPOCH ))
    if (( SECS > 0 )); then
      M=$(( SECS / 60 ))
      S=$(( SECS % 60 ))
      if (( M > 0 )); then
        DURATION="${M}m ${S}s"
      else
        DURATION="${S}s"
      fi
    fi
  fi
fi

# Construct the Slack payload using jq for safe JSON escaping
PAYLOAD=$(jq -n \
  --arg repo "$REPO" \
  --arg workflow "$WORKFLOW_NAME" \
  --arg conclusion "$CONCLUSION" \
  --arg sha "$SHORT_SHA" \
  --arg commit_url "$COMMIT_URL" \
  --arg url "$RUN_URL" \
  --arg job_name "$JOB_NAME" \
  --arg job_url "$JOB_HTML_URL" \
  --arg duration "$DURATION" \
  --arg trigger "$TRIGGER_EVENT" \
  '
  def field($t): { type: "mrkdwn", text: $t };
  def maybe_job_fields:
    if ($job_name | length) > 0 then
      ( [ field("*Job*\n\($job_name)") ]
        + (if ($job_url | length) > 0 then [ field("*Job Logs*\n<\($job_url)|Open logs>") ] else [] end) )
    else [] end;
  def maybe_field($name; $val): if ($val | length) > 0 then [ field("*\($name)*\n\($val)") ] else [] end;
  def maybe_link_field($name; $title; $url): if ($url | length) > 0 then [ field("*\($name)*\n<\($url)|\($title)>") ] else [] end;

  {
    blocks: [
      {
        type: "section",
        text: { type: "mrkdwn", text: ":rotating_light: *GitHub Actions job failed*" }
      },
      {
        type: "section",
        fields: (
          [ field("*Workflow*\n\($workflow)"),
            field("*Conclusion*\n\($conclusion)")
          ]
          + maybe_link_field("Commit"; $sha; $commit_url)
          + maybe_link_field("Run"; "Open in GitHub"; $url)
          + maybe_job_fields
            + maybe_field("Duration"; $duration)
            + maybe_field("Trigger"; $trigger)
        )
      }
    ]
  }
  ')
curl -sS --fail --connect-timeout 5 --max-time 15 -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL" || echo "Failed to send Slack notification" >&2
