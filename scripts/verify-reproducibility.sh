#!/bin/bash
# Verify reproducibility of guest program builds using ere-compiler.
#
# This script builds the same guest program twice using ere-compiler Docker
# images and compares the output hashes to verify reproducibility.
#
# Usage:
#   ./scripts/verify-reproducibility.sh [zkvm] [ere_version]
#
# Arguments:
#   zkvm        - Target zkVM: sp1, risc0, or zisk (default: zisk)
#   ere_version - ere-compiler version tag (default: latest)
#
# Examples:
#   ./scripts/verify-reproducibility.sh zisk latest
#   ./scripts/verify-reproducibility.sh sp1 0.2.0-abcd123
#   ./scripts/verify-reproducibility.sh risc0

set -e

ZKVM="${1:-zisk}"
ERE_VERSION="${2:-latest}"

echo "=============================================="
echo "Reproducibility Verification"
echo "=============================================="
echo "zkVM: $ZKVM"
echo "ere-compiler version: $ERE_VERSION"
echo "=============================================="

# Validate zkVM
case "$ZKVM" in
    sp1|risc0|zisk)
        ;;
    *)
        echo "Error: Invalid zkVM '$ZKVM'. Must be one of: sp1, risc0, zisk"
        exit 1
        ;;
esac

# Create temp directories
BUILD1_DIR=$(mktemp -d)
BUILD2_DIR=$(mktemp -d)
trap "rm -rf $BUILD1_DIR $BUILD2_DIR" EXIT

WORKSPACE=$(pwd)

echo ""
echo "=== Build 1 ==="
docker run --rm \
    -v "$WORKSPACE:/workspace:ro" \
    -v "$BUILD1_DIR:/output" \
    "ghcr.io/eth-act/ere/ere-compiler-$ZKVM:$ERE_VERSION" \
    --compiler-kind rust-customized \
    --guest-path "/workspace/crates/guest-program/bin/$ZKVM" \
    --output-path /output/ethrex-$ZKVM

echo ""
echo "=== Build 2 ==="
docker run --rm \
    -v "$WORKSPACE:/workspace:ro" \
    -v "$BUILD2_DIR:/output" \
    "ghcr.io/eth-act/ere/ere-compiler-$ZKVM:$ERE_VERSION" \
    --compiler-kind rust-customized \
    --guest-path "/workspace/crates/guest-program/bin/$ZKVM" \
    --output-path /output/ethrex-$ZKVM

echo ""
echo "=== Comparing Hashes ==="

HASH1=$(sha256sum "$BUILD1_DIR/ethrex-$ZKVM" | cut -d' ' -f1)
HASH2=$(sha256sum "$BUILD2_DIR/ethrex-$ZKVM" | cut -d' ' -f1)

echo "Build 1 SHA256: $HASH1"
echo "Build 2 SHA256: $HASH2"

if [ "$HASH1" = "$HASH2" ]; then
    echo ""
    echo "=============================================="
    echo "PASS: Builds are reproducible!"
    echo "SHA256: $HASH1"
    echo "=============================================="
    exit 0
else
    echo ""
    echo "=============================================="
    echo "FAIL: Builds differ!"
    echo "Build 1: $HASH1"
    echo "Build 2: $HASH2"
    echo "=============================================="
    exit 1
fi
