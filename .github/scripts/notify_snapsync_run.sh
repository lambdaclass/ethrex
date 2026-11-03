#!/usr/bin/env bash
set -euo pipefail

# Usage: notify_snapsync_run.sh
# Expects the following env vars (provided by the caller workflow):
#   SLACK_WEBHOOK_URL_SUCCESS, SLACK_WEBHOOK_URL_FAILURE, REPO, NAME, OUTCOME,
#   HEAD_SHA, START_TIME, RUN_ID, RUN_ATTEMPT
# Optional:
#   COMMIT_MESSAGE  (falls back to `git log` of HEAD_SHA when unset)
#   EVENT_NAME      (github.event_name; labels HEAD_SHA honestly on scheduled runs)
#   GH_TOKEN        (needs actions:read; adds this workflow's earlier conclusions
#                    on the same commit)

REPO=${REPO:-}
NAME=${NAME:-}
OUTCOME=${OUTCOME:-}
HEAD_SHA=${HEAD_SHA:-}
START_TIME=${START_TIME:-}
RUN_ID=${RUN_ID:-}
RUN_ATTEMPT=${RUN_ATTEMPT:-1}
COMMIT_MESSAGE=${COMMIT_MESSAGE:-}
EVENT_NAME=${EVENT_NAME:-}
GH_TOKEN=${GH_TOKEN:-}

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

# On a scheduled run HEAD_SHA is whatever main pointed at when the run started,
# not a change under test. Label it that way and show the commit's age: a
# failure on a two-day-old commit that ran fine in between is a flaky run, not
# a regression, and the alert should not read as a case for reverting it.
COMMIT_LABEL="Commit"
if [[ "$EVENT_NAME" == "schedule" ]]; then
  COMMIT_LABEL="Head of main at run start"
fi

COMMIT_AGE=""
if [[ -n "$HEAD_SHA" ]]; then
  COMMIT_TS=$(git log -1 --format=%ct "$HEAD_SHA" 2>/dev/null || true)
  if [[ "$COMMIT_TS" =~ ^[0-9]+$ ]] && (( EPOCHSECONDS >= COMMIT_TS )); then
    AGE_SECS=$((EPOCHSECONDS - COMMIT_TS))
    COMMIT_AGE=" (committed $((AGE_SECS / 86400))d $(((AGE_SECS % 86400) / 3600))h ago)"
  fi
fi

if [[ -n "$HEAD_SHA" ]]; then
  SHORT_SHA="${HEAD_SHA:0:8}"
  COMMIT_URL="https://github.com/${REPO}/commit/${HEAD_SHA}"
  COMMIT_LINE="*${COMMIT_LABEL}:* <${COMMIT_URL}|${SHORT_SHA}>"
  if [[ -n "$COMMIT_TITLE" ]]; then
    COMMIT_LINE="${COMMIT_LINE} $(slack_escape "$COMMIT_TITLE")"
  fi
  COMMIT_LINE="${COMMIT_LINE}${COMMIT_AGE}"
else
  COMMIT_LINE="*${COMMIT_LABEL}:* unknown"
fi

# This workflow's earlier conclusions on the same commit. "success ×6,
# failure ×1" is the quickest way to tell a flaky run from a broken commit.
# Omitted when no token is provided.
HISTORY_LINE=""
if [[ -n "$GH_TOKEN" && -n "$HEAD_SHA" && -n "$REPO" && -n "${GITHUB_WORKFLOW:-}" ]]; then
  prior=$(curl -sS --max-time 20 \
      -H "Authorization: Bearer ${GH_TOKEN}" -H "Accept: application/vnd.github+json" \
      "https://api.github.com/repos/${REPO}/actions/runs?head_sha=${HEAD_SHA}&per_page=100" 2>/dev/null \
    | jq -r --arg wf "$GITHUB_WORKFLOW" --arg id "$RUN_ID" '
        [.workflow_runs[]? | select(.name == $wf and (.id|tostring) != $id and .conclusion != null) | .conclusion]
        | group_by(.) | map("\(.[0]) ×\(length)") | join(", ")' 2>/dev/null || true)
  HISTORY_LINE="*Earlier runs on this commit:* ${prior:-none}"
fi

DETAILS="${COMMIT_LINE}"$'\n'"*Duration:* ${DURATION}"
if [[ -n "$HISTORY_LINE" ]]; then
  DETAILS="${DETAILS}"$'\n'"${HISTORY_LINE}"
fi

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
