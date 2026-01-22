#!/usr/bin/env bash
set -euo pipefail

REPORT_FILE="$1"

curl -X POST "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/sendMessage" \
  -d chat_id="$TELEGRAM_ETHREX_CHAT_ID" \
  -d parse_mode=HTML \
  --data-urlencode text="$(cat "$REPORT_FILE" | sed '/```/d' | sed '1s/^# \(.*\)$/<b>\1<\/b>/' | sed 's/^\* /• /')"
