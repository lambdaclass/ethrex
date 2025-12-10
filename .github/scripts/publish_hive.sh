#!/usr/bin/env bash
set -euo pipefail

WEBHOOK_URL="$1"
BLOCKS_FILE="$2"
RUN_URL="${3:-}"

PAYLOAD=$(jq -c --arg run_url "$RUN_URL" '
  .blocks += (if ($run_url | length) > 0 then [
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": {
            "type": "plain_text",
            "text": "View full breakdown"
          },
          "url": $run_url
        }
      ]
    }
  ] else [] end)
' "$BLOCKS_FILE")

printf '%s' "$PAYLOAD" | curl -X POST "$WEBHOOK_URL" \
-H 'Content-Type: application/json; charset=utf-8' \
--data @-
