#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_workflow_failure.sh
# Expects the following env vars (provided by the caller workflow):
#   SLACK_WEBHOOK_URL, REPO, WORKFLOW_NAME, CONCLUSION, EVENT, RUN_HTML_URL,
#   RUN_ID, RUN_ATTEMPT, HEAD_SHA, COMMIT_MESSAGE, FAILED_JOBS

# A missing webhook (e.g. on forks, where secrets are unavailable) is not an
# error; failing to deliver to a configured webhook is.
if [[ -z "${SLACK_WEBHOOK_URL:-}" ]]; then
  echo "Slack webhook URL not provided; skipping notification." >&2
  exit 0
fi

REPO=${REPO:-}
WORKFLOW_NAME=${WORKFLOW_NAME:-}
CONCLUSION=${CONCLUSION:-failure}
EVENT=${EVENT:-push}
RUN_HTML_URL=${RUN_HTML_URL:-}
RUN_ID=${RUN_ID:-}
RUN_ATTEMPT=${RUN_ATTEMPT:-1}
HEAD_SHA=${HEAD_SHA:-}
COMMIT_MESSAGE=${COMMIT_MESSAGE:-}
FAILED_JOBS=${FAILED_JOBS:-Unknown job}

if ! [[ "$RUN_ATTEMPT" =~ ^[0-9]+$ ]]; then
  RUN_ATTEMPT=1
fi

# Escape the characters that are special in Slack mrkdwn text. Uses sed
# because in bash >= 5.2 an unquoted `&` in a ${var//pat/repl} replacement
# expands to the matched text instead of a literal ampersand.
slack_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

RUN_URL="$RUN_HTML_URL"
if [[ -z "$RUN_URL" ]]; then
  RUN_URL="https://github.com/${REPO}/actions/runs/${RUN_ID}"
fi

case "$CONCLUSION" in
  timed_out) VERB="timed out" ;;
  startup_failure) VERB="failed to start" ;;
  *) VERB="failed" ;;
esac

# Scheduled runs are not caused by the commit at the head of main, so don't
# present them as a failure introduced "on main".
if [[ "$EVENT" == "schedule" ]]; then
  HEADLINE="Scheduled workflow $(slack_escape "$WORKFLOW_NAME") ${VERB}"
else
  HEADLINE="$(slack_escape "$WORKFLOW_NAME") ${VERB} on main"
fi
if (( RUN_ATTEMPT > 1 )); then
  HEADLINE="${HEADLINE} (attempt ${RUN_ATTEMPT})"
fi

if [[ -n "$HEAD_SHA" ]]; then
  SHORT_SHA="${HEAD_SHA:0:8}"
  COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"
  COMMIT_LINE="*Commit:* <${COMMIT_URL}|${SHORT_SHA}>"
  COMMIT_TITLE=$(printf '%s' "$COMMIT_MESSAGE" | head -n 1)
  if [[ -n "$COMMIT_TITLE" ]]; then
    COMMIT_LINE="${COMMIT_LINE} $(slack_escape "$COMMIT_TITLE")"
  fi
else
  COMMIT_LINE="*Commit:* unknown"
fi

# Construct the Slack payload using jq for safe JSON escaping
PAYLOAD=$(jq -n \
  --arg headline ":rotating_light: *<${RUN_URL}|${HEADLINE}>*" \
  --arg commit "$COMMIT_LINE" \
  --arg failed_jobs "*Failed job(s)*"$'\n'"$FAILED_JOBS" \
  '{
    blocks: [
      { type: "section", text: { type: "mrkdwn", text: $headline } },
      { type: "section", text: { type: "mrkdwn", text: $commit } },
      { type: "section", text: { type: "mrkdwn", text: $failed_jobs } }
    ]
  }')

# Let delivery failures fail the job so lost alerts are visible in the
# Actions tab instead of being silently swallowed.
curl -sS --fail --retry 3 -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$SLACK_WEBHOOK_URL"
