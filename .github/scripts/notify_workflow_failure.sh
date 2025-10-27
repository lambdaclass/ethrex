#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_workflow_failure.sh <slack_webhook_url>
# Expects the following env vars (provided by the caller workflow):
#   REPO, WORKFLOW_NAME, CONCLUSION, RUN_HTML_URL, RUN_ID, HEAD_SHA

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
FAILED_JOBS=${FAILED_JOBS:-Unknown job}

RUN_URL="$RUN_HTML_URL"
if [[ -z "$RUN_URL" ]]; then
  RUN_URL="https://github.com/${REPO}/actions/runs/${RUN_ID}"
fi

SHORT_SHA="${HEAD_SHA:0:8}"
COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"

# Construct the Slack payload using jq for safe JSON escaping
PAYLOAD=$(jq -n \
  --arg repo "$REPO" \
  --arg workflow "$WORKFLOW_NAME" \
  --arg conclusion "$CONCLUSION" \
  --arg sha "$SHORT_SHA" \
  --arg commit_url "$COMMIT_URL" \
  --arg url "$RUN_URL" \
  --arg failed_jobs "$FAILED_JOBS" \
  '{
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: ":rotating_light: *Workflow failed on main*"
        }
      },
      {
        type: "section",
        fields: [
          { type: "mrkdwn", text: "*Workflow*\n\($workflow)" },
          { type: "mrkdwn", text: "*Conclusion*\n\($conclusion)" },
          { type: "mrkdwn", text: "*Commit*\n<\($commit_url)|\($sha)>" },
          { type: "mrkdwn", text: "*Run*\n<\($url)|Open in GitHub>" },
          { type: "mrkdwn", text: "*Failed job(s)*\n\($failed_jobs)" }
        ]
      }
    ]
  }')
curl -sS --fail -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL" || echo "Failed to send Slack notification" >&2
