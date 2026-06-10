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

# For transaction-level coverage, run the hive build-block simulator
# (make run-hive-build-block) — it drives txs via the fixtures. To fill the
# mempool here for a manual check, point tooling/load_test at $RPC.

echo ">> done. ethrex log: /tmp/ethrex-bb.log"
