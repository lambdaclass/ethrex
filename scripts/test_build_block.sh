#!/usr/bin/env bash
# Local smoke test for testing_buildBlockV1 + debug_setHead.
# Boots ethrex on an Amsterdam (BAL) genesis, builds an empty block on top of
# genesis via testing_buildBlockV1, then exercises debug_setHead.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ETHREX_BIN:-$ROOT/target/debug/ethrex}"
GENESIS="${GENESIS:-$ROOT/fixtures/genesis/l1-bal.json}"
PORT="${PORT:-8545}"
RPC="http://127.0.0.1:$PORT"
DATADIR="$(mktemp -d "${TMPDIR:-/tmp}/ethrex-bb.XXXXXX")"

rpc() { # method, params-json
  curl -s "$RPC" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"
}

echo ">> starting ethrex ($BIN) on $GENESIS"
"$BIN" --network "$GENESIS" --datadir "$DATADIR" \
  --http.addr 127.0.0.1 --http.port "$PORT" \
  --http.api eth,testing,debug >/tmp/ethrex-bb.log 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATADIR"' EXIT

echo ">> waiting for RPC"
for _ in $(seq 1 60); do
  if rpc eth_blockNumber '[]' | grep -q result; then break; fi
  sleep 0.5
done

GENESIS_HASH=$(rpc eth_getBlockByNumber '["0x0", false]' | jq -r '.result.hash')
GENESIS_TS=$(rpc eth_getBlockByNumber '["0x0", false]' | jq -r '.result.timestamp')
echo ">> genesis hash=$GENESIS_HASH ts=$GENESIS_TS"

NEXT_TS=$(printf '0x%x' $(( $(printf '%d' "$GENESIS_TS") + 12 )))
ATTRS=$(jq -nc --arg ts "$NEXT_TS" '{
  timestamp: $ts,
  prevRandao: "0x0000000000000000000000000000000000000000000000000000000000000000",
  suggestedFeeRecipient: "0x0000000000000000000000000000000000000000",
  withdrawals: [],
  parentBeaconBlockRoot: "0x0000000000000000000000000000000000000000000000000000000000000000"
}')

echo ">> testing_buildBlockV1 (empty block on genesis)"
RES=$(rpc testing_buildBlockV1 "[\"$GENESIS_HASH\", $ATTRS, [], \"0x\"]")
echo "$RES" | jq '{
  err: .error,
  number: .result.executionPayload.blockNumber,
  parent: .result.executionPayload.parentHash,
  stateRoot: .result.executionPayload.stateRoot,
  txs: (.result.executionPayload.transactions | length),
  hasBAL: (.result.executionPayload.blockAccessList != null),
  blockValue: .result.blockValue
}'

echo ">> debug_setHead(0x0)"
rpc debug_setHead '["0x0"]' | jq '{err: .error, result: .result}'

# Transaction-level coverage: submit signed txs to the mempool, then build a
# non-empty block from them via testing_buildBlockV1 (transactions: null).
# Set SKIP_TX=1 to skip (avoids needing the prebuilt example binary).
SENDER_BIN="${SENDER_BIN:-$ROOT/target/debug/examples/send_to_mempool}"
RICH_KEY="${RICH_KEY:-0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e}"
if [ "${SKIP_TX:-0}" != "1" ] && [ -x "$SENDER_BIN" ]; then
  echo ">> submitting 2 txs to mempool"
  "$SENDER_BIN" "$RPC" "$RICH_KEY" 2
  echo ">> testing_buildBlockV1 (transactions:null -> include mempool txs)"
  rpc testing_buildBlockV1 "[\"$GENESIS_HASH\", $ATTRS, null]" | jq '{
    err: .error,
    number: .result.executionPayload.blockNumber,
    txs: (.result.executionPayload.transactions | length),
    gasUsed: .result.executionPayload.gasUsed,
    stateRoot: .result.executionPayload.stateRoot,
    balBytes: (.result.executionPayload.blockAccessList | length),
    blockValue: .result.blockValue
  }'
else
  echo ">> skipping tx phase (build with: cargo build -p ethrex-sdk --example send_to_mempool)"
fi

echo ">> done. ethrex log: /tmp/ethrex-bb.log"
