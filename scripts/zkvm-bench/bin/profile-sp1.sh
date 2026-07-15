#!/bin/bash
# scripts/zkvm-bench/profile-sp1.sh
# Generate SP1 execution statistics using ethrex-replay
#
# Prerequisites:
# - ethrex-replay: https://github.com/lambdaclass/ethrex-replay
# - SP1 guest program built

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

BLOCK=${1:-""}
OUTPUT_DIR=${2:-"$BENCH_ROOT/profiles/sp1"}
RPC_URL=${3:-"${RPC_URL:-http://localhost:8545}"}
DESCRIPTION=${4:-""}  # Optional description for filename
ETHREX_REPLAY_PATH=${ETHREX_REPLAY_PATH:-"$REPO_ROOT/../ethrex-replay"}

if [ -z "$BLOCK" ]; then
    echo "Usage: $0 <block_number> [output_dir] [rpc_url] [description]"
    echo ""
    echo "Arguments:"
    echo "  block_number - Block number to profile (required)"
    echo "  output_dir   - Directory for stats output (default: profiles/sp1)"
    echo "  rpc_url      - RPC endpoint URL (default: \$RPC_URL or http://localhost:8545)"
    echo "  description  - Optional description for filename"
    echo ""
    echo "Environment:"
    echo "  RPC_URL              - Default RPC endpoint"
    echo "  ETHREX_REPLAY_PATH   - Path to ethrex-replay repo (default: ../ethrex-replay)"
    echo ""
    echo "Example:"
    echo "  $0 24283607"
    echo "  $0 24283607 profiles/sp1 http://localhost:8545 'baseline'"
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
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Get git commit hash (short form)
COMMIT_HASH=$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo "unknown")

# Build filename: stats_<timestamp>_<commit>_<description>.txt
if [ -n "$DESCRIPTION" ]; then
    # Sanitize description: lowercase, replace spaces with underscores, remove special chars
    SANITIZED=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' | tr -cd '[:alnum:]_')
    STATS_FILE="$OUTPUT_DIR/stats_${TIMESTAMP}_${COMMIT_HASH}_${SANITIZED}.txt"
else
    STATS_FILE="$OUTPUT_DIR/stats_${TIMESTAMP}_${COMMIT_HASH}.txt"
fi

echo "Profiling SP1 execution..."
echo "Block: $BLOCK"
echo "RPC: $RPC_URL"
echo "Output: $STATS_FILE"
echo "Commit: $COMMIT_HASH"
if [ -n "$DESCRIPTION" ]; then
    echo "Description: $DESCRIPTION"
fi
echo ""

cd "$ETHREX_REPLAY_PATH"

# Build and run ethrex-replay with SP1 backend
echo "Running SP1 execution via ethrex-replay..."
cargo run -r --features sp1 -p ethrex-replay -- \
    block "$BLOCK" \
    --rpc-url "$RPC_URL" \
    --zkvm sp1 \
    --action execute \
    --verbose \
    2>&1 | tee "$STATS_FILE"

echo ""
echo "Statistics saved to: $STATS_FILE"
