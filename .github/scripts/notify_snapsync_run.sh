#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_snapsync_run.sh
# Expects the following env vars (provided by the caller workflow):
#   SLACK_WEBHOOK_URL_SUCCESS, SLACK_WEBHOOK_URL_FAILURE, REPO, NAME, OUTCOME, HEAD_SHA, START_TIME

if [[ -z "${SLACK_WEBHOOK_URL_SUCCESS}" ]]; then
  echo "Slack webhook URL not provided; skipping notification." >&2
  exit 0
fi

if [[ -z "${SLACK_WEBHOOK_URL_FAILURE}" ]]; then
  echo "Slack webhook URL not provided; skipping notification." >&2
  exit 0
fi

REPO=${REPO:-}
NAME=${NAME:-}
OUTCOME=${OUTCOME:-}
HEAD_SHA=${HEAD_SHA:-}
START_TIME=${START_TIME:-}

DURATION_SECS=$((EPOCHSECONDS - START_TIME))
DURATION=$(date -d@"$DURATION_SECS" -u +%H:%M:%S)

SHORT_SHA="${HEAD_SHA:0:8}"
COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"

# Construct the Slack payload using jq for safe JSON escaping
if [[ "$OUTCOME" == "success" ]]; then
  PAYLOAD=$(jq -n \
  --arg name "$NAME" \
  --arg sha "$SHORT_SHA" \
  --arg commit_url "$COMMIT_URL" \
  --arg duration "$DURATION" \
  '{
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: ":white_check_mark: Node snapsynced successfully"
        }
      },
      {
        type: "section",
        fields: [
          { type: "mrkdwn", text: "*Job*\n\($name)" },
          { type: "mrkdwn", text: "*Commit*\n<\($commit_url)|\($sha)>" },
          { type: "mrkdwn", text: "*Duration*\n\($duration)" }
        ]
      }
    ]
  }')
  curl -sS --fail -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL_SUCCESS" || echo "Failed to send Slack notification" >&2
else
  PAYLOAD=$(jq -n \
  --arg name "$NAME" \
  --arg sha "$SHORT_SHA" \
  --arg commit_url "$COMMIT_URL" \
  --arg duration "$DURATION" \
  '{
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: ":rotating_light: *Node failed to snapsync*"
        }
      },
      {
        type: "section",
        fields: [
          { type: "mrkdwn", text: "*Job*\n\($name)" },
          { type: "mrkdwn", text: "*Commit*\n<\($commit_url)|\($sha)>" },
          { type: "mrkdwn", text: "*Duration*\n\($duration)" }
        ]
      }
    ]
  }')
  curl -sS --fail -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL_FAILURE" || echo "Failed to send Slack notification" >&2
fi
