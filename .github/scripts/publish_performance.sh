#!/usr/bin/env bash
set -euo pipefail

WEBHOOK_URL="$1"
PAYLOAD_FILE="$2"

curl -X POST "$WEBHOOK_URL" \
  -H 'Content-Type: application/json; charset=utf-8' \
  --data @"$PAYLOAD_FILE"
