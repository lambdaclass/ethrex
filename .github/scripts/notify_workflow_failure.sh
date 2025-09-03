#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_workflow_failure.sh <slack_webhook_url>
# Expects the following env vars (provided by the caller workflow):
#   REPO, WORKFLOW_NAME, CONCLUSION, RUN_HTML_URL, RUN_ID, HEAD_SHA, ACTOR

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
ACTOR=${ACTOR:-}

RUN_URL="$RUN_HTML_URL"
if [[ -z "$RUN_URL" ]]; then
  RUN_URL="https://github.com/${REPO}/actions/runs/${RUN_ID}"
fi

SHORT_SHA="${HEAD_SHA:0:8}"

# Construct the Slack payload using jq for safe JSON escaping
PAYLOAD=$(jq -n \
  --arg repo "$REPO" \
  --arg workflow "$WORKFLOW_NAME" \
  --arg conclusion "$CONCLUSION" \
  --arg actor "$ACTOR" \
  --arg sha "$SHORT_SHA" \
  --arg url "$RUN_URL" \
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
          { type: "mrkdwn", text: "*Repo*\n\($repo)" },
          { type: "mrkdwn", text: "*Workflow*\n\($workflow)" },
          { type: "mrkdwn", text: "*Conclusion*\n\($conclusion)" },
          { type: "mrkdwn", text: "*Actor*\n\($actor)" },
          { type: "mrkdwn", text: "*Commit*\n\($sha)" },
          { type: "mrkdwn", text: "*Run*\n<\($url)|Open in GitHub>" }
        ]
      }
    ]
  }')
curl -sS -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL"

