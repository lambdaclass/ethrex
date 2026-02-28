#!/bin/bash
# scripts/zkvm-bench/run-bench.sh
# Run benchmarks on mainnet blocks using ethrex-replay
#
# Prerequisites:
# - ethrex-replay: https://github.com/lambdaclass/ethrex-replay
# - RPC endpoint supporting debug_executionWitness

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

ZKVM=${1:-zisk}  # Default to ZisK (most optimized)
BLOCK_NUM=${2:-""}
RPC_URL=${3:-"http://localhost:8545"}
ACTION=${4:-execute}  # execute or prove
ETHREX_REPLAY_PATH=${ETHREX_REPLAY_PATH:-"$REPO_ROOT/../ethrex-replay"}

if [ -z "$BLOCK_NUM" ]; then
    echo "Usage: $0 <zkvm> <block_number> [rpc_url] [action]"
    echo ""
    echo "Arguments:"
    echo "  zkvm         - Backend to use: sp1 or zisk (default: zisk)"
    echo "  block_number - Ethereum block number to benchmark (required)"
    echo "  rpc_url      - RPC endpoint URL (default: http://localhost:8545)"
    echo "  action       - Action to perform: execute or prove (default: execute)"
    echo ""
    echo "Environment:"
    echo "  ETHREX_REPLAY_PATH - Path to ethrex-replay repo (default: ../ethrex-replay)"
    echo ""
    echo "Examples:"
    echo "  $0 zisk 23769082 http://localhost:8545"
    echo "  $0 sp1 23769082 \$RPC_URL prove"
    echo ""
    echo "Note: The RPC endpoint must support debug_executionWitness"
    exit 1
fi

OUTPUT_DIR="$BENCH_ROOT/benchmarks/$ZKVM/$(date +%Y%m%d)"
mkdir -p "$OUTPUT_DIR"

echo "Running $ZKVM $ACTION on block $BLOCK_NUM"
echo "RPC: $RPC_URL"
echo "Output: $OUTPUT_DIR"
echo ""

if [ ! -d "$ETHREX_REPLAY_PATH" ]; then
    echo "Error: ethrex-replay not found at $ETHREX_REPLAY_PATH"
    echo ""
    echo "Clone it from: https://github.com/lambdaclass/ethrex-replay"
    echo "  git clone https://github.com/lambdaclass/ethrex-replay $ETHREX_REPLAY_PATH"
    echo ""
    echo "Or set ETHREX_REPLAY_PATH environment variable to its location"
    exit 1
fi

cd "$ETHREX_REPLAY_PATH"

# Build features based on zkVM
case $ZKVM in
  sp1)
    FEATURES="sp1"
    ;;
  zisk)
    FEATURES="zisk"
    ;;
  *)
    echo "Error: Unknown zkVM: $ZKVM (use sp1 or zisk)"
    exit 1
    ;;
esac

LOG_FILE="$OUTPUT_DIR/block_${BLOCK_NUM}_${ACTION}.log"

echo "Starting benchmark..."
echo "Log: $LOG_FILE"
echo ""

# Run ethrex-replay
cargo run -r -F "$FEATURES" -p ethrex-replay -- \
  blocks \
  --action "$ACTION" \
  --zkvm "$ZKVM" \
  --from "$BLOCK_NUM" \
  --to "$BLOCK_NUM" \
  --rpc-url "$RPC_URL" \
  2>&1 | tee "$LOG_FILE"

echo ""
echo "Benchmark complete!"
echo "Log saved to: $LOG_FILE"

# Extract key metrics if available
if grep -q "cycles" "$LOG_FILE" 2>/dev/null; then
    echo ""
    echo "=== Key Metrics ==="
    grep -E "cycles|steps|time|proof" "$LOG_FILE" | head -10
fi
