#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR=$(
  cd "$(dirname "${BASH_SOURCE[0]}")/../.."
  pwd -P
)
cd "$ROOT_DIR"

BIN="${BIN:-target/release/ethrex}"
RUNS="${RUNS:-3}"
WARMUP="${WARMUP:-1}"
NETWORK_FILE="${NETWORK_FILE:-fixtures/genesis/perf-ci.json}"
CHAIN_FILE="${CHAIN_FILE:-fixtures/blockchain/l2-1k-erc20.rlp}"
OUT_JSON="${OUT_JSON:-/tmp/ethrex_opt_matrix_$(date -u +%Y%m%dT%H%M%SZ).json}"

if [[ ! -x "$BIN" ]]; then
  echo "Building release binary: $BIN"
  cargo build --release -p ethrex --bin ethrex
fi

COMMON_CMD="$BIN --network $NETWORK_FILE --force import $CHAIN_FILE --removedb"

ROCKSDB_ENV="ETHREX_PERF_ROCKSDB_BLOCK_CACHE_MB=512 ETHREX_PERF_ROCKSDB_CACHE_INDEX_FILTER=1 ETHREX_PERF_ROCKSDB_PIN_L0_INDEX_FILTER=1 ETHREX_PERF_ROCKSDB_OPTIMIZE_FILTERS_FOR_HITS=1"
VM_CACHE_ENV="ETHREX_PERF_VM_READ_CACHE=1"
PIPELINE_ENV="ETHREX_PERF_PIPELINE_FLUSH_AFTER_TXS=3 ETHREX_PERF_PIPELINE_MAX_QUEUE_FOR_FLUSH=2"

hyperfine -w "$WARMUP" -r "$RUNS" --export-json "$OUT_JSON" \
  --command-name baseline \
  "$COMMON_CMD" \
  --command-name rocksdb \
  "$ROCKSDB_ENV $COMMON_CMD" \
  --command-name vm_cache \
  "$VM_CACHE_ENV $COMMON_CMD" \
  --command-name pipeline \
  "$PIPELINE_ENV $COMMON_CMD" \
  --command-name rocksdb_vm_cache \
  "$ROCKSDB_ENV $VM_CACHE_ENV $COMMON_CMD" \
  --command-name rocksdb_pipeline \
  "$ROCKSDB_ENV $PIPELINE_ENV $COMMON_CMD" \
  --command-name vm_cache_pipeline \
  "$VM_CACHE_ENV $PIPELINE_ENV $COMMON_CMD" \
  --command-name all_three \
  "$ROCKSDB_ENV $VM_CACHE_ENV $PIPELINE_ENV $COMMON_CMD"

echo
echo "Saved hyperfine JSON: $OUT_JSON"

python3 - <<'PY' "$OUT_JSON"
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)

results = data["results"]
baseline = next(r for r in results if r["command"] == "baseline")["mean"]

print("variant\tmean_s\tstddev_s\tvs_baseline_pct")
for r in results:
    mean = r["mean"]
    stddev = r["stddev"]
    delta = (mean / baseline - 1.0) * 100.0
    print(f"{r['command']}\t{mean:.3f}\t{stddev:.3f}\t{delta:+.2f}%")
PY
