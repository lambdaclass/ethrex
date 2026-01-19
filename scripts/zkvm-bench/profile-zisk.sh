#!/bin/bash
# scripts/zkvm-bench/profile-zisk.sh
# Generate ZisK execution statistics using ziskemu
#
# Prerequisites:
# - ziskemu: Part of ZisK toolchain (cargo-zisk sdk install-toolchain)
# - ZisK guest program built

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

INPUT_FILE=${1:-""}
OUTPUT_DIR=${2:-"$REPO_ROOT/profiles/zisk"}
TOP_ROI=${3:-25}  # Number of top functions to show
ELF_PATH="${4:-$REPO_ROOT/crates/l2/prover/src/guest_program/src/zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program}"

if [ -z "$INPUT_FILE" ]; then
    echo "Usage: $0 <input_file> [output_dir] [top_roi] [elf_path]"
    echo ""
    echo "Arguments:"
    echo "  input_file  - Path to the input .bin file (required)"
    echo "  output_dir  - Directory for stats output (default: profiles/zisk)"
    echo "  top_roi     - Number of top functions to display (default: 25)"
    echo "  elf_path    - Path to ELF file (default: guest program output)"
    echo ""
    echo "Example:"
    echo "  $0 inputs/ethrex_mainnet_23769082_input.bin"
    echo "  $0 inputs/block.bin profiles/zisk 50"
    exit 1
fi

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: Input file not found: $INPUT_FILE"
    exit 1
fi

if [ ! -f "$ELF_PATH" ]; then
    echo "Error: ELF file not found: $ELF_PATH"
    echo ""
    echo "Build the ZisK guest program first:"
    echo "  ./build.sh zisk"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
STATS_FILE="$OUTPUT_DIR/stats_$TIMESTAMP.txt"

echo "Profiling ZisK execution..."
echo "Input: $INPUT_FILE"
echo "ELF: $ELF_PATH"
echo "Output: $STATS_FILE"
echo "Top functions: $TOP_ROI"
echo ""

# Run ziskemu with full statistics
# -X: Generate opcode/memory statistics
# -S: Load symbols from ELF for function names
# -D: Show detailed analysis per function (top callers)
# -T: Number of top functions to display
ziskemu \
  -e "$ELF_PATH" \
  -i "$INPUT_FILE" \
  -X \
  -S \
  -D \
  -T "$TOP_ROI" \
  2>&1 | tee "$STATS_FILE"

echo ""
echo "Statistics saved to: $STATS_FILE"
echo ""

# Quick summary for terminal
echo "=== Quick Summary ==="
grep -E "^STEPS|^COST DISTRIBUTION|^TOP COST FUNCTIONS" -A 12 "$STATS_FILE" 2>/dev/null | head -30 || true
echo ""
echo "For full details, see: $STATS_FILE"
echo "Convert to JSON: ./to-json.sh $STATS_FILE"
