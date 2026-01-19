#!/bin/bash
#
# Mock Consensus Benchmark Script
#
# This script starts an ethrex node with a large genesis file and runs
# the mock consensus client to benchmark Engine API performance.
#
# Usage:
#   ./run_mock_consensus_bench.sh [NUM_BLOCKS] [TXS_PER_BLOCK] [SLOT_TIME_MS]
#
# Examples:
#   ./run_mock_consensus_bench.sh           # 100 blocks, 400 txs/block, 1000ms slots
#   ./run_mock_consensus_bench.sh 50        # 50 blocks
#   ./run_mock_consensus_bench.sh 100 200   # 100 blocks, 200 txs/block

set -e

# Configuration
NUM_BLOCKS="${1:-100}"
TXS_PER_BLOCK="${2:-400}"
SLOT_TIME_MS="${3:-1000}"
MAX_ACCOUNTS="${4:-10000}"

# Paths (relative to repo root)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ETHREX_BIN="./target/release/ethrex"
MOCK_CONSENSUS_BIN="./target/release/mock_consensus"
GENESIS_FILE="test_data/genesis_1m/genesis.json"
KEYS_FILE="test_data/genesis_1m/private_keys.txt"
DATA_DIR="dev_data"
JWT_SECRET_FILE="jwt.hex"
OUTPUT_FILE="timing_results_$(date +%Y%m%d_%H%M%S).csv"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================"
echo "  Mock Consensus Benchmark"
echo "========================================"
echo ""
echo "Configuration:"
echo "  Blocks: $NUM_BLOCKS"
echo "  Txs/Block: $TXS_PER_BLOCK"
echo "  Slot Time: ${SLOT_TIME_MS}ms"
echo "  Max Accounts: $MAX_ACCOUNTS"
echo "  Output: $OUTPUT_FILE"
echo ""

# Step 1: Check if binaries exist, build if necessary
echo -e "${YELLOW}[1/6] Checking binaries...${NC}"
if [ ! -f "$ETHREX_BIN" ]; then
    echo "Building ethrex..."
    cargo build --bin ethrex --release --features dev
fi

if [ ! -f "$MOCK_CONSENSUS_BIN" ]; then
    echo "Building mock_consensus..."
    cargo build -p ethrex-benches --bin mock_consensus --release
fi
echo -e "${GREEN}Binaries ready.${NC}"

# Step 2: Check genesis and keys files
echo -e "${YELLOW}[2/6] Checking genesis and keys files...${NC}"
if [ ! -f "$GENESIS_FILE" ]; then
    echo -e "${RED}Error: Genesis file not found at $GENESIS_FILE${NC}"
    echo "Run: cargo run -p ethrex-benches --bin generate_genesis --release -- --output test_data/genesis_1m"
    exit 1
fi

if [ ! -f "$KEYS_FILE" ]; then
    echo -e "${RED}Error: Keys file not found at $KEYS_FILE${NC}"
    exit 1
fi
echo -e "${GREEN}Genesis and keys files found.${NC}"

# Step 3: Create JWT secret if not exists
echo -e "${YELLOW}[3/6] Setting up JWT secret...${NC}"
if [ ! -f "$JWT_SECRET_FILE" ]; then
    echo "0x$(openssl rand -hex 32)" > "$JWT_SECRET_FILE"
    echo "Created new JWT secret."
else
    # Ensure it has 0x prefix
    if ! grep -q "^0x" "$JWT_SECRET_FILE"; then
        echo "0x$(cat $JWT_SECRET_FILE)" > "$JWT_SECRET_FILE"
    fi
    echo "Using existing JWT secret."
fi

# Step 4: Clean data directory
echo -e "${YELLOW}[4/6] Cleaning data directory...${NC}"
rm -rf "$DATA_DIR"
echo -e "${GREEN}Data directory cleaned.${NC}"

# Step 5: Start the node
echo -e "${YELLOW}[5/6] Starting ethrex node...${NC}"
echo "This may take a while for genesis trie construction..."

# Start node in background, capture output
NODE_LOG="node_output_$(date +%Y%m%d_%H%M%S).log"
$ETHREX_BIN \
    --dev \
    --dev.no-blocks \
    --network "$GENESIS_FILE" \
    --datadir "$DATA_DIR" \
    --force \
    --authrpc.jwtsecret "$JWT_SECRET_FILE" \
    > "$NODE_LOG" 2>&1 &

NODE_PID=$!
echo "Node started with PID: $NODE_PID"
echo "Node log: $NODE_LOG"

# Function to cleanup on exit
cleanup() {
    echo ""
    echo -e "${YELLOW}Cleaning up...${NC}"
    if [ -n "$NODE_PID" ] && kill -0 "$NODE_PID" 2>/dev/null; then
        echo "Stopping node (PID: $NODE_PID)..."
        kill "$NODE_PID" 2>/dev/null || true
        wait "$NODE_PID" 2>/dev/null || true
    fi
    echo -e "${GREEN}Cleanup complete.${NC}"
}
trap cleanup EXIT

# Wait for node to be ready (check RPC endpoint)
echo "Waiting for node to be ready..."
MAX_WAIT=120
WAITED=0
while [ $WAITED -lt $MAX_WAIT ]; do
    if curl -s http://localhost:8545 -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' 2>/dev/null | grep -q "result"; then
        echo -e "${GREEN}Node is ready!${NC}"
        break
    fi
    sleep 2
    WAITED=$((WAITED + 2))
    # Show progress from log
    if [ -f "$NODE_LOG" ]; then
        LAST_LINE=$(tail -1 "$NODE_LOG" 2>/dev/null || echo "")
        if [ -n "$LAST_LINE" ]; then
            echo -ne "\r  $LAST_LINE                    "
        fi
    fi
done
echo ""

if [ $WAITED -ge $MAX_WAIT ]; then
    echo -e "${RED}Error: Node did not start within $MAX_WAIT seconds${NC}"
    echo "Last 20 lines of node log:"
    tail -20 "$NODE_LOG"
    exit 1
fi

# Show genesis timing from log
echo ""
echo "Genesis initialization times:"
grep -E "(parsing took|trie.*completed)" "$NODE_LOG" || true
echo ""

# Step 6: Run mock consensus benchmark
echo -e "${YELLOW}[6/6] Running mock consensus benchmark...${NC}"
echo ""

$MOCK_CONSENSUS_BIN \
    --node-url "http://localhost:8545" \
    --auth-url "http://localhost:8551" \
    --jwt-secret "$JWT_SECRET_FILE" \
    --keys-file "$KEYS_FILE" \
    --num-blocks "$NUM_BLOCKS" \
    --txs-per-block "$TXS_PER_BLOCK" \
    --slot-time "$SLOT_TIME_MS" \
    --max-accounts "$MAX_ACCOUNTS" \
    --output "$OUTPUT_FILE"

BENCH_EXIT_CODE=$?

echo ""
echo "========================================"
echo "  Benchmark Complete"
echo "========================================"
echo ""
echo "Results saved to: $OUTPUT_FILE"
echo "Node log saved to: $NODE_LOG"
echo ""

if [ $BENCH_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}Benchmark completed successfully!${NC}"
else
    echo -e "${RED}Benchmark failed with exit code: $BENCH_EXIT_CODE${NC}"
fi

# Show quick stats from CSV
if [ -f "$OUTPUT_FILE" ]; then
    echo ""
    echo "Quick stats from CSV:"
    echo "  Total records: $(wc -l < "$OUTPUT_FILE")"
    echo "  Successful calls: $(grep -c ",true," "$OUTPUT_FILE" || echo 0)"
    echo "  Failed calls: $(grep -c ",false," "$OUTPUT_FILE" || echo 0)"
fi

exit $BENCH_EXIT_CODE
