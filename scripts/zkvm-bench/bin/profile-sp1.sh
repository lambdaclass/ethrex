#!/bin/bash
# scripts/zkvm-bench/profile-sp1.sh
# Generate SP1 flamegraph profile
#
# Prerequisites:
# - samply: cargo install --locked samply
# - SP1 guest program built with profiling feature

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

INPUT_FILE=${1:-""}
OUTPUT_DIR=${2:-"$BENCH_ROOT/profiles/sp1"}
SAMPLE_RATE=${3:-100}  # Higher = smaller file, less detail
DESCRIPTION=${4:-""}  # Optional description for filename

if [ -z "$INPUT_FILE" ]; then
    echo "Usage: $0 <input_file> [output_dir] [sample_rate] [description]"
    echo ""
    echo "Arguments:"
    echo "  input_file   - Path to the input .bin file (required)"
    echo "  output_dir   - Directory for profile output (default: profiles/sp1)"
    echo "  sample_rate  - Sample rate for tracing (default: 100, higher = less detail)"
    echo "  description  - Optional description for filename"
    echo ""
    echo "Example:"
    echo "  $0 inputs/ethrex_mainnet_23769082_input.bin"
    echo "  $0 inputs/block.bin profiles/sp1 100 'baseline'"
    exit 1
fi

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: Input file not found: $INPUT_FILE"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

if [ -n "$DESCRIPTION" ]; then
    SANITIZED=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '_' | tr -cd '[:alnum:]_')
    TRACE_FILE="$OUTPUT_DIR/trace_${TIMESTAMP}_${SANITIZED}.json"
else
    TRACE_FILE="$OUTPUT_DIR/trace_$TIMESTAMP.json"
fi

echo "Profiling SP1 execution..."
echo "Input: $INPUT_FILE"
echo "Output: $TRACE_FILE"
echo "Sample rate: 1 in every $SAMPLE_RATE cycles"
if [ -n "$DESCRIPTION" ]; then
    echo "Description: $DESCRIPTION"
fi
echo ""

# Build with profiling feature if not already
cd "$REPO_ROOT/crates/l2/prover"
echo "Building prover with profiling feature..."
cargo build --release --features "l2,sp1,profiling"

# Execute with tracing enabled
echo ""
echo "Running execution with tracing..."
TRACE_FILE="$TRACE_FILE" \
TRACE_SAMPLE_RATE="$SAMPLE_RATE" \
cargo run --release --features "l2,sp1,profiling" -- \
  execute --input "$INPUT_FILE"

echo ""
echo "Profile saved to: $TRACE_FILE"
echo ""
echo "To view the profile, run:"
echo "  samply load $TRACE_FILE"
echo ""
echo "Or open http://127.0.0.1:8000/ui/flamegraph after running samply"
