#!/usr/bin/env bash
set -euo pipefail

REPORT_FILE="$1"

if [[ -z "${TELEGRAM_BOT_TOKEN:-}" ]]; then
  echo "::error::TELEGRAM_BOT_TOKEN secret is not set — skipping Telegram post"
  exit 1
fi

if [[ -z "${TELEGRAM_ETHREX_CHAT_ID:-}" ]]; then
  echo "::error::TELEGRAM_ETHREX_CHAT_ID resolved to an empty value — check that the appropriate secret is configured for this trigger (scheduled vs manual)"
  exit 1
fi

curl -X POST "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/sendMessage" \
  -d chat_id="$TELEGRAM_ETHREX_CHAT_ID" \
  -d parse_mode=HTML \
  --data-urlencode text="$(cat "$REPORT_FILE")"
