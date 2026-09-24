#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_snapsync_run.sh
# Expects the following env vars (provided by the caller workflow):
#   SLACK_WEBHOOK_URL_SUCCESS, SLACK_WEBHOOK_URL_FAILURE, REPO, NAME, OUTCOME,
#   HEAD_SHA, START_TIME, RUN_ID, RUN_ATTEMPT
# Optional:
#   COMMIT_MESSAGE  (falls back to `git log` of HEAD_SHA when unset)

REPO=${REPO:-}
NAME=${NAME:-}
OUTCOME=${OUTCOME:-}
HEAD_SHA=${HEAD_SHA:-}
START_TIME=${START_TIME:-}
RUN_ID=${RUN_ID:-}
RUN_ATTEMPT=${RUN_ATTEMPT:-1}
COMMIT_MESSAGE=${COMMIT_MESSAGE:-}

if ! [[ "$RUN_ATTEMPT" =~ ^[0-9]+$ ]]; then
  RUN_ATTEMPT=1
fi

# Outcome decides both the destination channel and how the run is presented.
# A cancellation (manual stop, runner death) is not a sync failure, so it gets
# a distinct, lower-alarm headline while still going to the failure channel for
# visibility.
case "$OUTCOME" in
  success)
    WEBHOOK=${SLACK_WEBHOOK_URL_SUCCESS:-}
    HEADLINE_EMOJI=":white_check_mark:"
    HEADLINE_VERB="succeeded"
    ;;
  cancelled)
    WEBHOOK=${SLACK_WEBHOOK_URL_FAILURE:-}
    HEADLINE_EMOJI=":warning:"
    HEADLINE_VERB="was cancelled"
    ;;
  *)
    WEBHOOK=${SLACK_WEBHOOK_URL_FAILURE:-}
    HEADLINE_EMOJI=":rotating_light:"
    HEADLINE_VERB="failed"
    ;;
esac

# A missing webhook (e.g. on forks, where secrets are unavailable) is not an
# error; failing to deliver to a configured webhook is.
if [[ -z "$WEBHOOK" ]]; then
  echo "Slack webhook URL not provided for outcome '$OUTCOME'; skipping notification." >&2
  exit 0
fi

# Escape the characters that are special in Slack mrkdwn text. Uses sed
# because in bash >= 5.2 an unquoted `&` in a ${var//pat/repl} replacement
# expands to the matched text instead of a literal ampersand.
slack_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

DURATION="unknown"
if [[ "$START_TIME" =~ ^[0-9]+$ ]]; then
  DURATION_SECS=$((EPOCHSECONDS - START_TIME))
  if (( DURATION_SECS >= 0 )); then
    DURATION=$(date -d@"$DURATION_SECS" -u +%H:%M:%S)
  fi
fi

RUN_URL="https://github.com/${REPO}/actions/runs/${RUN_ID}"
if (( RUN_ATTEMPT > 1 )); then
  RUN_URL="${RUN_URL}/attempts/${RUN_ATTEMPT}"
fi

HEADLINE="Snapsync ${HEADLINE_VERB}: $(slack_escape "$NAME")"
if (( RUN_ATTEMPT > 1 )); then
  HEADLINE="${HEADLINE} (attempt ${RUN_ATTEMPT})"
fi

# Prefer an explicitly provided commit subject; otherwise read it from the
# checkout the job already has.
COMMIT_TITLE="$COMMIT_MESSAGE"
if [[ -z "$COMMIT_TITLE" && -n "$HEAD_SHA" ]]; then
  COMMIT_TITLE=$(git log -1 --format=%s "$HEAD_SHA" 2>/dev/null || true)
fi
COMMIT_TITLE=$(printf '%s' "$COMMIT_TITLE" | head -n 1)

if [[ -n "$HEAD_SHA" ]]; then
  SHORT_SHA="${HEAD_SHA:0:8}"
  COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"
  COMMIT_LINE="*Commit:* <${COMMIT_URL}|${SHORT_SHA}>"
  if [[ -n "$COMMIT_TITLE" ]]; then
    COMMIT_LINE="${COMMIT_LINE} $(slack_escape "$COMMIT_TITLE")"
  fi
else
  COMMIT_LINE="*Commit:* unknown"
fi

DETAILS="${COMMIT_LINE}"$'\n'"*Duration:* ${DURATION}"

# Construct the Slack payload using jq for safe JSON escaping
PAYLOAD=$(jq -n \
  --arg headline "${HEADLINE_EMOJI} *<${RUN_URL}|${HEADLINE}>*" \
  --arg details "$DETAILS" \
  '{
    blocks: [
      { type: "section", text: { type: "mrkdwn", text: $headline } },
      { type: "section", text: { type: "mrkdwn", text: $details } }
    ]
  }')

# Let delivery failures fail the job so lost notifications are visible in the
# Actions tab instead of being silently swallowed.
curl -sS --fail --retry 3 -X POST \
  -H 'Content-type: application/json' \
  --data "$PAYLOAD" \
  "$WEBHOOK"
