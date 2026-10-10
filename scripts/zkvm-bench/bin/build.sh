#!/bin/bash
# scripts/zkvm-bench/build.sh
# Quick rebuild for SP1 or ZisK guest programs

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
GUEST_DIR="$REPO_ROOT/crates/l2/prover/src/guest_program"

ZKVM=${1:-sp1}  # Default to SP1

case $ZKVM in
  sp1)
    echo "Building SP1 guest program..."
    cd "$GUEST_DIR/src/sp1"
    # Use incremental builds when possible
    cargo build --release
    echo ""
    echo "Output: $GUEST_DIR/src/sp1/target/release/"
    ;;
  zisk)
    echo "Building ZisK guest program..."
    cd "$GUEST_DIR/src/zisk"
    cargo-zisk build --release
    echo ""
    echo "Output: $GUEST_DIR/src/zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program"
    ;;
  both)
    "$0" sp1
    "$0" zisk
    ;;
  *)
    echo "Usage: $0 [sp1|zisk|both]"
    echo ""
    echo "Options:"
    echo "  sp1   - Build SP1 guest program (default)"
    echo "  zisk  - Build ZisK guest program"
    echo "  both  - Build both guest programs"
    exit 1
    ;;
esac

echo ""
echo "Build complete for $ZKVM"
