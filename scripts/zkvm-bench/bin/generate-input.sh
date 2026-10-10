#!/bin/bash
# scripts/zkvm-bench/generate-input.sh
# Generate block execution inputs using ethrex-replay
#
# Prerequisites:
# - ethrex-replay: https://github.com/lambdaclass/ethrex-replay
# - RPC endpoint supporting debug_executionWitness

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

BLOCK=${1:-""}
RPC_URL=${2:-"${RPC_URL:-http://localhost:8545}"}
OUTPUT_DIR=${3:-"$BENCH_ROOT/inputs"}
ETHREX_REPLAY_PATH=${ETHREX_REPLAY_PATH:-"$REPO_ROOT/../ethrex-replay"}

if [ -z "$BLOCK" ]; then
    echo "Usage: $0 <block_number> [rpc_url] [output_dir]"
    echo ""
    echo "Generate block execution input using ethrex-replay."
    echo ""
    echo "Arguments:"
    echo "  block_number - Ethereum block number (required)"
    echo "  rpc_url      - RPC endpoint URL (default: \$RPC_URL or http://localhost:8545)"
    echo "  output_dir   - Output directory (default: scripts/zkvm-bench/inputs)"
    echo ""
    echo "Environment:"
    echo "  RPC_URL              - Default RPC endpoint"
    echo "  ETHREX_REPLAY_PATH   - Path to ethrex-replay repo (default: ../ethrex-replay)"
    echo ""
    echo "Examples:"
    echo "  $0 23769082"
    echo "  $0 23769082 http://localhost:8545"
    echo "  RPC_URL=\$ALCHEMY_URL $0 23769082"
    exit 1
fi

if [ ! -d "$ETHREX_REPLAY_PATH" ]; then
    echo "Error: ethrex-replay not found at $ETHREX_REPLAY_PATH"
    echo ""
    echo "Clone it from: https://github.com/lambdaclass/ethrex-replay"
    echo "  git clone https://github.com/lambdaclass/ethrex-replay $ETHREX_REPLAY_PATH"
    echo ""
    echo "Or set ETHREX_REPLAY_PATH environment variable"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

# Get git commit hash (short form)
COMMIT_HASH=$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo "unknown")

# Output filename includes commit hash to track witness version
OUTPUT_FILE="$OUTPUT_DIR/ethrex_mainnet_${BLOCK}_${COMMIT_HASH}_input.bin"

# Check if input already exists for this commit
if [ -f "$OUTPUT_FILE" ]; then
    echo "Input already exists: $OUTPUT_FILE"
    echo "Delete it to regenerate."
    exit 0
fi

echo "Generating input for block $BLOCK..."
echo "RPC: $RPC_URL"
echo "Commit: $COMMIT_HASH"
echo "Output: $OUTPUT_FILE"
echo ""

cd "$ETHREX_REPLAY_PATH"

# ethrex-replay generates a fixed filename, rename afterward
TEMP_OUTPUT="$OUTPUT_DIR/ethrex_mainnet_${BLOCK}_input.bin"

cargo run -r -p ethrex-replay -- \
    generate-input \
    --block "$BLOCK" \
    --rpc-url "$RPC_URL" \
    --output-dir "$OUTPUT_DIR"

# Rename to include commit hash
if [ -f "$TEMP_OUTPUT" ]; then
    mv "$TEMP_OUTPUT" "$OUTPUT_FILE"
fi

echo ""
echo "Input generated: $OUTPUT_FILE"
