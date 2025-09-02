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

read -r -d '' PAYLOAD <<'JSON'
{
  "blocks": [
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": ":rotating_light: *Workflow failed on main*"
      }
    },
    {
      "type": "section",
      "fields": [
        { "type": "mrkdwn", "text": "*Repo*\nREPO_VAL" },
        { "type": "mrkdwn", "text": "*Workflow*\nWORKFLOW_VAL" },
        { "type": "mrkdwn", "text": "*Conclusion*\nCONCLUSION_VAL" },
        { "type": "mrkdwn", "text": "*Actor*\nACTOR_VAL" },
        { "type": "mrkdwn", "text": "*Commit*\nSHA_VAL" },
        { "type": "mrkdwn", "text": "*Run*\n<URL_VAL|Open in GitHub>" }
      ]
    }
  ]
}
JSON

PAYLOAD=${PAYLOAD/REPO_VAL/${REPO}}
PAYLOAD=${PAYLOAD/WORKFLOW_VAL/${WORKFLOW_NAME}}
PAYLOAD=${PAYLOAD/CONCLUSION_VAL/${CONCLUSION}}
PAYLOAD=${PAYLOAD/ACTOR_VAL/${ACTOR}}
PAYLOAD=${PAYLOAD/SHA_VAL/${SHORT_SHA}}

# Escape URL safely for substitution
ESCAPED_URL=$(printf '%s' "$RUN_URL" | sed 's/[&/]/\\&/g')
PAYLOAD=$(printf '%s' "$PAYLOAD" | sed "s/URL_VAL/$ESCAPED_URL/")

curl -sS -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL"

