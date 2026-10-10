#!/bin/bash
# scripts/zkvm-bench/profile-zisk.sh
# Generate ZisK execution statistics using ziskemu
#
# Prerequisites:
# - ziskemu: Part of ZisK toolchain (cargo-zisk sdk install-toolchain)
# - ZisK guest program built

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

INPUT_FILE=${1:-""}
OUTPUT_DIR=${2:-"$BENCH_ROOT/profiles/zisk"}
TOP_ROI=${3:-25}  # Number of top functions to show
DESCRIPTION=${4:-""}  # Optional description for filename
ELF_PATH="${5:-$REPO_ROOT/crates/l2/prover/src/guest_program/src/zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program}"

if [ -z "$INPUT_FILE" ]; then
    echo "Usage: $0 <input_file> [output_dir] [top_roi] [description] [elf_path]"
    echo ""
    echo "Arguments:"
    echo "  input_file  - Path to the input .bin file (required)"
    echo "  output_dir  - Directory for stats output (default: profiles/zisk)"
    echo "  top_roi     - Number of top functions to display (default: 25)"
    echo "  description  - Optional description for filename (sanitized: lowercase, underscores for spaces)"
    echo "  elf_path    - Path to ELF file (default: guest program output)"
    echo ""
    echo "Example:"
    echo "  $0 inputs/ethrex_mainnet_23769082_input.bin"
    echo "  $0 inputs/block.bin profiles/zisk 50 'baseline'"
    echo "  $0 inputs/block.bin profiles/zisk 50 'decode_child_opt'"
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

echo "Profiling ZisK execution..."
echo "Input: $INPUT_FILE"
echo "ELF: $ELF_PATH"
echo "Output: $STATS_FILE"
echo "Top functions: $TOP_ROI"
echo "Commit: $COMMIT_HASH"
if [ -n "$DESCRIPTION" ]; then
    echo "Description: $DESCRIPTION"
fi
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
