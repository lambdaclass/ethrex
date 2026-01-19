#!/bin/bash
# Performance benchmark script for ethrex optimizations
# Usage: ./benches/run_perf_benchmark.sh [label] [num_blocks] [txs_per_block]

set -e

LABEL="${1:-baseline}"
NUM_BLOCKS="${2:-50}"
TXS_PER_BLOCK="${3:-400}"
SLOT_TIME="${4:-1000}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

# Output files
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
TIMING_FILE="timing_${LABEL}_${TIMESTAMP}.csv"
NODE_LOG="node_${LABEL}_${TIMESTAMP}.log"
RESULTS_FILE="results_${LABEL}_${TIMESTAMP}.txt"

echo "========================================"
echo "  Performance Benchmark: $LABEL"
echo "========================================"
echo "  Blocks: $NUM_BLOCKS"
echo "  Txs/block: $TXS_PER_BLOCK"
echo "  Slot time: ${SLOT_TIME}ms"
echo "========================================"

# Cleanup function
cleanup() {
    echo "Cleaning up..."
    if [ -f node_bench.pid ]; then
        kill $(cat node_bench.pid) 2>/dev/null || true
        rm -f node_bench.pid
    fi
}
trap cleanup EXIT

# Kill any existing node
if [ -f node_bench.pid ]; then
    kill $(cat node_bench.pid) 2>/dev/null || true
    sleep 2
fi

# Clean data directory
rm -rf dev_data

# Create JWT secret if needed
if [ ! -f jwt_bench.hex ]; then
    echo "0x$(openssl rand -hex 32)" > jwt_bench.hex
fi

# Check prerequisites
if [ ! -f "test_data/genesis_1m/genesis.json" ]; then
    echo "ERROR: Genesis file not found. Run generate_genesis first."
    exit 1
fi

if [ ! -f "target/release/ethrex" ]; then
    echo "Building ethrex..."
    cargo build --bin ethrex --release --features dev
fi

if [ ! -f "target/release/mock_consensus" ]; then
    echo "Building mock_consensus..."
    cargo build -p ethrex-benches --bin mock_consensus --release
fi

# Start node
echo "Starting ethrex node..."
./target/release/ethrex \
    --dev \
    --dev.no-blocks \
    --network test_data/genesis_1m/genesis.json \
    --datadir dev_data \
    --force \
    --authrpc.jwtsecret jwt_bench.hex > "$NODE_LOG" 2>&1 &

echo $! > node_bench.pid
echo "Node PID: $(cat node_bench.pid)"

# Wait for node to be ready
echo "Waiting for node to be ready..."
for i in {1..60}; do
    if curl -s http://localhost:8545 > /dev/null 2>&1; then
        echo "Node ready after ${i}s"
        break
    fi
    if [ $i -eq 60 ]; then
        echo "ERROR: Node failed to start"
        cat "$NODE_LOG"
        exit 1
    fi
    sleep 1
done

# Run benchmark
echo "Running benchmark..."
./target/release/mock_consensus \
    --node-url http://localhost:8545 \
    --auth-url http://localhost:8551 \
    --jwt-secret jwt_bench.hex \
    --keys-file test_data/genesis_1m/private_keys.txt \
    --num-blocks "$NUM_BLOCKS" \
    --txs-per-block "$TXS_PER_BLOCK" \
    --slot-time "$SLOT_TIME" \
    --max-accounts 10000 \
    --output "$TIMING_FILE" 2>&1 | tee "$RESULTS_FILE"

# Extract metrics from node log
echo ""
echo "========================================"
echo "  Node Internal Metrics (last 10 blocks)"
echo "========================================"
grep "METRIC" "$NODE_LOG" | tail -10

# Stop node
kill $(cat node_bench.pid) 2>/dev/null || true
rm -f node_bench.pid

echo ""
echo "========================================"
echo "  Output Files"
echo "========================================"
echo "  Timing CSV: $TIMING_FILE"
echo "  Node log: $NODE_LOG"
echo "  Results: $RESULTS_FILE"
echo "========================================"
