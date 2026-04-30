#!/usr/bin/env bash
#
# poll-sync.sh — Two-phase sync completion detector for ethrex snap sync.
#
# Phase 1: Wait until eth_syncing returns a syncing object with highestBlock > 0.
#           Connection errors and "false" responses are retried (node not ready yet).
# Phase 2: Wait until eth_syncing returns false (sync complete).
#           Logs currentBlock progress periodically.
# Validation: After sync, calls eth_blockNumber and verifies it's > 0.
#
# On timeout or failure, outputs tail of ethrex/Lighthouse logs to $GITHUB_STEP_SUMMARY.
#
# Usage:
#   poll-sync.sh --endpoint URL --timeout DURATION --poll-interval SECS \
#                [--ethrex-log PATH] [--lighthouse-log PATH]

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
ENDPOINT="http://localhost:8545"
TIMEOUT="3h"
POLL_INTERVAL=60
ETHREX_LOG=""
LIGHTHOUSE_LOG=""

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint)        ENDPOINT="$2";        shift 2 ;;
    --timeout)         TIMEOUT="$2";         shift 2 ;;
    --poll-interval)   POLL_INTERVAL="$2";   shift 2 ;;
    --ethrex-log)      ETHREX_LOG="$2";      shift 2 ;;
    --lighthouse-log)  LIGHTHOUSE_LOG="$2";  shift 2 ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Convert human-readable timeout (e.g. "3h", "90m", "5400") to seconds
# ---------------------------------------------------------------------------
parse_timeout() {
  local raw="$1"
  if [[ "$raw" =~ ^([0-9]+)h$ ]]; then
    echo $(( ${BASH_REMATCH[1]} * 3600 ))
  elif [[ "$raw" =~ ^([0-9]+)m$ ]]; then
    echo $(( ${BASH_REMATCH[1]} * 60 ))
  elif [[ "$raw" =~ ^([0-9]+)s?$ ]]; then
    echo "${BASH_REMATCH[1]}"
  else
    echo "Error: cannot parse timeout '$raw'. Use e.g. '3h', '90m', or '5400'." >&2
    exit 1
  fi
}

TIMEOUT_SECS=$(parse_timeout "$TIMEOUT")
START_TIME=$(date +%s)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
elapsed() {
  echo $(( $(date +%s) - START_TIME ))
}

check_timeout() {
  if (( $(elapsed) >= TIMEOUT_SECS )); then
    echo "ERROR: Timeout of ${TIMEOUT} (${TIMEOUT_SECS}s) exceeded after $(elapsed)s."
    dump_logs
    exit 1
  fi
}

# Dump logs on failure to $GITHUB_STEP_SUMMARY (if available) and stderr.
dump_logs() {
  local summary="${GITHUB_STEP_SUMMARY:-}"

  {
    echo ""
    echo "### Snap Sync Failure Logs"
    echo ""

    if [[ -n "$ETHREX_LOG" && -f "$ETHREX_LOG" ]]; then
      echo "<details><summary>ethrex (last 200 lines)</summary>"
      echo ""
      echo '```'
      tail -n 200 "$ETHREX_LOG"
      echo '```'
      echo "</details>"
      echo ""
    fi

    if [[ -n "$LIGHTHOUSE_LOG" && -f "$LIGHTHOUSE_LOG" ]]; then
      echo "<details><summary>Lighthouse (last 100 lines)</summary>"
      echo ""
      echo '```'
      tail -n 100 "$LIGHTHOUSE_LOG"
      echo '```'
      echo "</details>"
      echo ""
    fi
  } | if [[ -n "$summary" ]]; then
    tee -a "$summary" >&2
  else
    cat >&2
  fi
}

# JSON-RPC helper. Returns the raw JSON response or empty string on curl error.
rpc_call() {
  local method="$1"
  curl -s --max-time 10 \
    -X POST "$ENDPOINT" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}" \
    2>/dev/null || true
}

# Extract .result from JSON-RPC response (requires jq-free parsing for portability,
# but jq is available on all GitHub runners).
rpc_result() {
  local method="$1"
  local response
  response=$(rpc_call "$method")
  if [[ -z "$response" ]]; then
    echo ""
    return
  fi
  echo "$response" | python3 -c "import sys,json; r=json.load(sys.stdin); print(json.dumps(r.get('result','')))" 2>/dev/null || echo ""
}

# ---------------------------------------------------------------------------
# Phase 1: Wait for sync to start (highestBlock > 0)
# ---------------------------------------------------------------------------
echo "=== Phase 1: Waiting for sync to start ==="
echo "Endpoint: ${ENDPOINT}"
echo "Timeout:  ${TIMEOUT} (${TIMEOUT_SECS}s)"
echo ""

while true; do
  check_timeout

  result=$(rpc_result "eth_syncing")

  # Connection error or empty result — node not ready yet
  if [[ -z "$result" || "$result" == '""' ]]; then
    echo "[$(date -u '+%H:%M:%S')] Waiting for node to respond... ($(elapsed)s elapsed)"
    sleep "$POLL_INTERVAL"
    continue
  fi

  # false means "not syncing" — node hasn't started sync yet
  if [[ "$result" == "false" ]]; then
    echo "[$(date -u '+%H:%M:%S')] Node responded false (sync not started yet). ($(elapsed)s elapsed)"
    sleep "$POLL_INTERVAL"
    continue
  fi

  # Parse syncing object — check if highestBlock > 0
  highest_block=$(echo "$result" | python3 -c "
import sys, json
data = json.load(sys.stdin)
if isinstance(data, dict):
    hb = data.get('highestBlock', '0x0')
    print(int(hb, 16) if isinstance(hb, str) and hb.startswith('0x') else int(hb))
else:
    print(0)
" 2>/dev/null || echo "0")

  if (( highest_block > 0 )); then
    current_block=$(echo "$result" | python3 -c "
import sys, json
data = json.load(sys.stdin)
if isinstance(data, dict):
    cb = data.get('currentBlock', '0x0')
    print(int(cb, 16) if isinstance(cb, str) and cb.startswith('0x') else int(cb))
else:
    print(0)
" 2>/dev/null || echo "0")
    echo "[$(date -u '+%H:%M:%S')] Sync started! currentBlock=${current_block} highestBlock=${highest_block}"
    break
  fi

  echo "[$(date -u '+%H:%M:%S')] Syncing object received but highestBlock=0. Waiting... ($(elapsed)s elapsed)"
  sleep "$POLL_INTERVAL"
done

# ---------------------------------------------------------------------------
# Phase 2: Wait for sync to complete (eth_syncing returns false)
# ---------------------------------------------------------------------------
echo ""
echo "=== Phase 2: Waiting for sync to complete ==="
echo ""

while true; do
  check_timeout

  result=$(rpc_result "eth_syncing")

  # Sync complete
  if [[ "$result" == "false" ]]; then
    echo "[$(date -u '+%H:%M:%S')] eth_syncing returned false — sync complete!"
    break
  fi

  # Log progress if we have a syncing object
  if [[ -n "$result" && "$result" != '""' ]]; then
    progress=$(echo "$result" | python3 -c "
import sys, json
data = json.load(sys.stdin)
if isinstance(data, dict):
    cb = data.get('currentBlock', '0x0')
    hb = data.get('highestBlock', '0x0')
    current = int(cb, 16) if isinstance(cb, str) and cb.startswith('0x') else int(cb)
    highest = int(hb, 16) if isinstance(hb, str) and hb.startswith('0x') else int(hb)
    pct = (current / highest * 100) if highest > 0 else 0
    print(f'currentBlock={current} highestBlock={highest} ({pct:.1f}%)')
else:
    print('unknown state')
" 2>/dev/null || echo "parse error")
    echo "[$(date -u '+%H:%M:%S')] ${progress} ($(elapsed)s elapsed)"
  else
    echo "[$(date -u '+%H:%M:%S')] No response from node. ($(elapsed)s elapsed)"
  fi

  sleep "$POLL_INTERVAL"
done

# ---------------------------------------------------------------------------
# Validation: eth_blockNumber > 0
# ---------------------------------------------------------------------------
echo ""
echo "=== Validation: checking eth_blockNumber ==="

block_hex=$(rpc_result "eth_blockNumber")

# Strip surrounding quotes if present
block_hex=$(echo "$block_hex" | tr -d '"')

if [[ -z "$block_hex" || "$block_hex" == "null" ]]; then
  echo "ERROR: eth_blockNumber returned empty/null after sync."
  dump_logs
  exit 1
fi

block_number=$(python3 -c "print(int('${block_hex}', 16))" 2>/dev/null || echo "0")

if (( block_number > 0 )); then
  echo "SUCCESS: Sync complete. Block number: ${block_number} (${block_hex})"
  exit 0
else
  echo "ERROR: Sync reported complete but eth_blockNumber is 0."
  dump_logs
  exit 1
fi
