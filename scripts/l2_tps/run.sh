#!/usr/bin/env bash
# One L2 TPS measurement: <backend> in {sp1,zisk,risc0,exec}, <workload> in
# {eth-transfers,erc20,fibonacci,io-heavy}. See docs/l2/tps-benchmarking.md.
#
# Brings up a fresh L1 dev node, the L1 contracts requiring <backend> proofs, the
# L2 sequencer, the prover and an endless load test, lets it run for DURATION_MIN
# minutes, then stops the load and the prover, dumps the per-block and per-batch
# data and leaves everything in a run directory. The L1 and L2 nodes keep running
# so they can be inspected; the next run stops them.
#
# Inputs (environment):
#   ETHREX_BIN       ethrex binary built with the backend (default target/release/ethrex)
#   LOAD_TEST_BIN    load test binary (default tooling/target/release/load_test)
#   TPS_RUNS_DIR     where run directories go (default <repo>/tps-runs)
#   DURATION_MIN     measurement window in minutes (default 15)
#   TX_PER_ACCOUNT   transactions per account and round (default 300)
#   BLOCK_TIME_MS    L2 block time (default 5000)
#   COMMIT_TIME_MS   committer interval (default = block time, so every block is batched at once)
#   ZISK_VERIFIER    for zisk: an already deployed ZiskVerifier address; otherwise
#                    `make deploy-zisk-verifier` deploys one
#   PROVER_ENV       extra environment for the prover, e.g. GLIBC_TUNABLES=... or
#                    LD_LIBRARY_PATH=... (space-separated VAR=value pairs)
set -euo pipefail

BACKEND=${1:?backend: sp1 | zisk | risc0 | exec}
WORKLOAD=${2:?workload: eth-transfers | erc20 | fibonacci | io-heavy}
DURATION_MIN=${DURATION_MIN:-15}
TX_PER_ACCOUNT=${TX_PER_ACCOUNT:-300}
BLOCK_TIME_MS=${BLOCK_TIME_MS:-5000}
COMMIT_TIME_MS=${COMMIT_TIME_MS:-$BLOCK_TIME_MS}

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BIN=${ETHREX_BIN:-$ROOT/target/release/ethrex}
LOAD_TEST=${LOAD_TEST_BIN:-$ROOT/tooling/target/release/load_test}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUN_DIR=${TPS_RUNS_DIR:-$ROOT/tps-runs}/$(date -u +%Y%m%dT%H%M%SZ)-$BACKEND-$WORKLOAD
mkdir -p "$RUN_DIR"
for v in $(env | grep -o '^ETHREX_[A-Z0-9_]*' | grep -v '^ETHREX_BIN$' || true); do unset "$v"; done

L1_RPC_URL=http://localhost:8545
L2_RPC_URL=http://localhost:1729
# Default test keys from fixtures/, worthless outside the local devnet.
L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
PROOF_SENDER_PRIVATE_KEY=0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
L2_OWNER_ADDRESS=0x4417092b70a3e5f10dc504d0947dd256b965fc62
BRIDGE_OWNER_PRIVATE_KEY=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e

log() { echo "== $(date -u +%FT%TZ) $*" | tee -a "$RUN_DIR/run.log"; }
rpc() { curl -s -X POST -H 'content-type: application/json' --data "$1" "$2"; }

stop_all() {
  pkill -f "$LOAD_TEST" || true
  pkill -f "$BIN" || true
  for _ in $(seq 1 30); do pgrep -f "$BIN" >/dev/null || break; sleep 1; done
  pkill -9 -f "$BIN" || true
  pkill -x sp1-gpu-server || true
  sleep 1
}

cd "$ROOT/crates/l2"
log "run $BACKEND/$WORKLOAD: duration ${DURATION_MIN} min, ${TX_PER_ACCOUNT} tx/account/round, block time ${BLOCK_TIME_MS} ms, commit time ${COMMIT_TIME_MS} ms"
log "ethrex $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown), $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1 || echo "no GPU")"
log "stopping leftovers and wiping datadirs"
stop_all
rm -rf "$ROOT/crates/l2/dev_ethrex_l1" "$ROOT/crates/l2/dev_ethrex_l2" "$ROOT/cmd/.env"

log "starting the L1 dev node"
nohup "$BIN" --network ../../fixtures/genesis/l1.json --http.port 8545 --http.addr 0.0.0.0 \
  --authrpc.port 8551 --dev --datadir dev_ethrex_l1 > "$RUN_DIR/l1.log" 2>&1 &
for _ in $(seq 1 60); do
  rpc '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' "$L1_RPC_URL" | grep -q result && break
  sleep 1
done

VERIFIER_FLAGS=()
case "$BACKEND" in
  zisk)
    if [ -z "${ZISK_VERIFIER:-}" ]; then
      log "deploying ZiskVerifier from the pinned ZisK release"
      make deploy-zisk-verifier > "$RUN_DIR/deploy-verifier.log" 2>&1
      ZISK_VERIFIER=$(grep -oE '0x[0-9a-fA-F]{40}' "$RUN_DIR/deploy-verifier.log" | tail -1)
    fi
    log "ZiskVerifier at $ZISK_VERIFIER"
    VERIFIER_FLAGS=(--zisk true --zisk.verifier-address "$ZISK_VERIFIER") ;;
  sp1) VERIFIER_FLAGS=(--sp1 true) ;;
  risc0) VERIFIER_FLAGS=(--risc0 true) ;;
  exec) VERIFIER_FLAGS=() ;;
  *) echo "unknown backend $BACKEND"; exit 1 ;;
esac

log "deploying the L1 contracts requiring $BACKEND proofs"
"$BIN" l2 deploy \
  --eth-rpc-url "$L1_RPC_URL" --private-key "$L1_PRIVATE_KEY" \
  "${VERIFIER_FLAGS[@]}" \
  --on-chain-proposer-owner "$L2_OWNER_ADDRESS" --bridge-owner "$L2_OWNER_ADDRESS" \
  --bridge-owner-pk "$BRIDGE_OWNER_PRIVATE_KEY" --deposit-rich \
  --private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
  --genesis-l1-path ../../fixtures/genesis/l1.json --genesis-l2-path ../../fixtures/genesis/l2.json \
  > "$RUN_DIR/deploy.log" 2>&1
set -a
# shellcheck disable=SC1091
. "$ROOT/cmd/.env"
set +a
OCP=$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS
log "bridge=$ETHREX_WATCHER_BRIDGE_ADDRESS on_chain_proposer=$OCP"

log "starting the L2 sequencer"
ETHREX_NO_MONITOR=true nohup "$BIN" l2 \
  --watcher.block-delay 0 --network ../../fixtures/genesis/l2.json \
  --http.port 1729 --http.addr 0.0.0.0 --metrics --metrics.port 3702 --datadir dev_ethrex_l2 \
  --mempool.maxsize 1000000 \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" --l1.on-chain-proposer-address "$OCP" \
  --eth.rpc-url "$L1_RPC_URL" --osaka-activation-time 1761677592 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
  --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
  --block-producer.operator-fee-per-gas 1000000000 \
  --block-producer.block-time "$BLOCK_TIME_MS" --committer.commit-time "$COMMIT_TIME_MS" \
  --committer.l1-private-key "$L1_PRIVATE_KEY" \
  --proof-coordinator.l1-private-key "$PROOF_SENDER_PRIVATE_KEY" --proof-coordinator.addr 127.0.0.1 \
  > "$RUN_DIR/l2.log" 2>&1 &
for _ in $(seq 1 60); do
  rpc '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' "$L2_RPC_URL" | grep -q result && break
  sleep 1
done

log "starting the load test ($WORKLOAD, endless)"
cd "$ROOT"
LOAD_TEST_RPC_URL=$L2_RPC_URL LOAD_TEST_TX_AMOUNT=$TX_PER_ACCOUNT LOAD_TEST_ENDLESS=true \
  nohup "$LOAD_TEST" -k ./fixtures/keys/private_keys.txt -t "$WORKLOAD" > "$RUN_DIR/loadtest.log" 2>&1 &
cd "$ROOT/crates/l2"

log "starting the $BACKEND prover"
# shellcheck disable=SC2086
env RUST_LOG=info,ethrex_prover=debug ${PROVER_ENV:-} nohup "$BIN" l2 prover --backend "$BACKEND" \
  --proof-coordinators tcp://127.0.0.1:3900 --timed > "$RUN_DIR/prover.log" 2>&1 &

log "sampling the OnChainProposer and the GPU every 30 s for ${DURATION_MIN} min"
echo "utc,committed,verified" > "$RUN_DIR/l1-progress.csv"
echo "utc,gpu_memory_used_mib,gpu_utilization_pct,gpu_power_w,host_mem_used_gib" > "$RUN_DIR/gpu.csv"
for _ in $(seq 1 $((DURATION_MIN * 2))); do
  sleep 30
  V=$(rex call "$OCP" "lastVerifiedBatch()" --rpc-url "$L1_RPC_URL" 2>/dev/null | tail -1 || echo 0x0)
  C=$(rex call "$OCP" "lastCommittedBatch()" --rpc-url "$L1_RPC_URL" 2>/dev/null | tail -1 || echo 0x0)
  GPU=$(nvidia-smi --query-gpu=memory.used,utilization.gpu,power.draw --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -d " " || echo ",,")
  HOSTMEM=$(free -g | awk "/^Mem:/ {print \$3}")
  echo "$(date -u +%FT%TZ),$((C)),$((V))" >> "$RUN_DIR/l1-progress.csv"
  echo "$(date -u +%FT%TZ),$GPU,$HOSTMEM" >> "$RUN_DIR/gpu.csv"
  echo "$(date -u +%T) committed=$((C)) verified=$((V)) gpu=$GPU" >> "$RUN_DIR/run.log"
done

log "stopping the load test and the prover, dumping metrics"
pkill -f "$LOAD_TEST" || true
curl -s localhost:3702/metrics > "$RUN_DIR/metrics.txt" || true
# Per-block series (timestamp, transactions, gas) straight from the L2 RPC.
python3 "$HERE/dump_blocks.py" "$RUN_DIR" "$L2_RPC_URL" >> "$RUN_DIR/run.log" 2>&1 || true
# L1 timestamps of every batch verification, for the settled TPS.
echo "first_batch,last_batch,verify_tx,l1_block,l1_timestamp" > "$RUN_DIR/verifications.csv"
grep -a "Sent batch verification transaction" "$RUN_DIR/l2.log" | while read -r line; do
  fb=$(echo "$line" | grep -o "first_batch=[0-9]*" | cut -d= -f2)
  lb=$(echo "$line" | grep -o "last_batch=[0-9]*" | cut -d= -f2)
  tx=$(echo "$line" | grep -o "verify_tx_hash=0x[0-9a-f]*" | cut -d= -f2)
  blk=$(rpc "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getTransactionReceipt\",\"params\":[\"$tx\"],\"id\":1}" "$L1_RPC_URL" \
    | python3 -c "import sys,json; r=json.load(sys.stdin).get(\"result\") or {}; print(r.get(\"blockNumber\",\"\"), r.get(\"status\",\"\"))")
  bn=${blk%% *}; st=${blk##* }
  [ -z "$bn" ] && continue
  ts=$(rpc "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBlockByNumber\",\"params\":[\"$bn\",false],\"id\":1}" "$L1_RPC_URL" \
    | python3 -c "import sys,json; print(int(json.load(sys.stdin)[\"result\"][\"timestamp\"],16))")
  [ "$st" = "0x1" ] && echo "$fb,$lb,$tx,$((bn)),$ts" >> "$RUN_DIR/verifications.csv"
done
pkill -f "$BIN l2 prover" || true
pkill -x sp1-gpu-server || true
python3 "$HERE/collect.py" "$RUN_DIR" > "$RUN_DIR/summary.md" 2>&1 || true
log "done: $RUN_DIR (L1 and L2 nodes left running; stop them with pkill -f $BIN)"
echo "$RUN_DIR"
