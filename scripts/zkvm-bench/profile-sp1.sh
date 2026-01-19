#!/bin/bash
# scripts/zkvm-bench/profile-sp1.sh
# Generate SP1 flamegraph profile
#
# Prerequisites:
# - samply: cargo install --locked samply
# - SP1 guest program built with profiling feature

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

INPUT_FILE=${1:-""}
OUTPUT_DIR=${2:-"$REPO_ROOT/profiles/sp1"}
SAMPLE_RATE=${3:-100}  # Higher = smaller file, less detail

if [ -z "$INPUT_FILE" ]; then
    echo "Usage: $0 <input_file> [output_dir] [sample_rate]"
    echo ""
    echo "Arguments:"
    echo "  input_file   - Path to the input .bin file (required)"
    echo "  output_dir   - Directory for profile output (default: profiles/sp1)"
    echo "  sample_rate  - Sample rate for tracing (default: 100, higher = less detail)"
    echo ""
    echo "Example:"
    echo "  $0 inputs/ethrex_mainnet_23769082_input.bin"
    exit 1
fi

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: Input file not found: $INPUT_FILE"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
TRACE_FILE="$OUTPUT_DIR/trace_$TIMESTAMP.json"

echo "Profiling SP1 execution..."
echo "Input: $INPUT_FILE"
echo "Output: $TRACE_FILE"
echo "Sample rate: 1 in every $SAMPLE_RATE cycles"
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
