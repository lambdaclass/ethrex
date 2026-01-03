#!/bin/bash
#
# Profile-Guided Optimization (PGO) Build Script for ethrex
#
# This script implements a three-phase PGO build process:
# 1. Instrumented build - Compile with profiling instrumentation
# 2. Profile collection - Run representative workloads to collect data
# 3. Optimized build - Rebuild using collected profile data
#
# Usage:
#   ./scripts/pgo-build.sh [phase]
#
# Phases:
#   instrument  - Build instrumented binary (phase 1)
#   collect     - Run workload to collect profile data (phase 2)
#   optimize    - Build optimized binary using profile data (phase 3)
#   all         - Run all phases sequentially (default)
#   clean       - Remove PGO artifacts
#
# Requirements:
#   - Rust nightly toolchain (for -Cprofile-generate/-Cprofile-use)
#   - llvm-profdata (usually comes with LLVM/clang)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PGO_DIR="$PROJECT_ROOT/target/pgo-profiles"
MERGED_PROFDATA="$PGO_DIR/merged.profdata"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

check_llvm_profdata() {
    if command -v llvm-profdata &> /dev/null; then
        LLVM_PROFDATA="llvm-profdata"
    elif command -v llvm-profdata-18 &> /dev/null; then
        LLVM_PROFDATA="llvm-profdata-18"
    elif command -v llvm-profdata-17 &> /dev/null; then
        LLVM_PROFDATA="llvm-profdata-17"
    elif command -v llvm-profdata-16 &> /dev/null; then
        LLVM_PROFDATA="llvm-profdata-16"
    else
        # Try to find any version
        LLVM_PROFDATA=$(find /usr/bin /usr/local/bin -name 'llvm-profdata*' 2>/dev/null | head -1)
        if [ -z "$LLVM_PROFDATA" ]; then
            error "llvm-profdata not found. Please install LLVM tools."
        fi
    fi
    info "Using: $LLVM_PROFDATA"
}

phase_instrument() {
    info "Phase 1: Building instrumented binary..."

    mkdir -p "$PGO_DIR"

    # Clean previous profile data
    rm -rf "$PGO_DIR"/*.profraw "$MERGED_PROFDATA"

    cd "$PROJECT_ROOT"

    # Build with instrumentation
    RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
        cargo build --release --bin ethrex

    success "Instrumented binary built: target/release/ethrex"
    info "Next step: Run './scripts/pgo-build.sh collect' or run your own workload with the instrumented binary"
}

phase_collect() {
    info "Phase 2: Collecting profile data..."

    if [ ! -f "$PROJECT_ROOT/target/release/ethrex" ]; then
        error "Instrumented binary not found. Run 'instrument' phase first."
    fi

    cd "$PROJECT_ROOT"

    # Run the EF state tests as the profiling workload
    # This exercises the hot paths in the EVM
    info "Running EF state tests as profiling workload..."

    if [ -d "$PROJECT_ROOT/tooling/ef_tests/state" ]; then
        # Build and run state tests with instrumentation
        RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
            cargo test --release -p ef_tests_state -- --test-threads=4 2>&1 || true
    fi

    # Alternative: Run blockchain tests
    if [ -d "$PROJECT_ROOT/tooling/ef_tests/blockchain" ]; then
        info "Running EF blockchain tests..."
        RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
            cargo test --release -p ef_tests_blockchain -- --test-threads=4 2>&1 || true
    fi

    # Check if we collected any profile data
    PROFRAW_COUNT=$(find "$PGO_DIR" -name "*.profraw" 2>/dev/null | wc -l)

    if [ "$PROFRAW_COUNT" -eq 0 ]; then
        warn "No .profraw files found. Trying with benchmarks..."

        # Try running the revm comparison benchmark if available
        if [ -d "$PROJECT_ROOT/crates/vm/levm/bench/revm_comparison" ]; then
            info "Running LEVM benchmarks..."
            cd "$PROJECT_ROOT/crates/vm/levm/bench/revm_comparison"
            RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
                cargo run --release 2>&1 || true
        fi
    fi

    PROFRAW_COUNT=$(find "$PGO_DIR" -name "*.profraw" 2>/dev/null | wc -l)
    if [ "$PROFRAW_COUNT" -eq 0 ]; then
        error "No profile data collected. Please run a workload manually with the instrumented binary."
    fi

    success "Collected $PROFRAW_COUNT profile data files"
    info "Next step: Run './scripts/pgo-build.sh optimize'"
}

phase_merge() {
    info "Merging profile data..."

    check_llvm_profdata

    PROFRAW_FILES=$(find "$PGO_DIR" -name "*.profraw")

    if [ -z "$PROFRAW_FILES" ]; then
        error "No .profraw files found in $PGO_DIR"
    fi

    $LLVM_PROFDATA merge -o "$MERGED_PROFDATA" $PROFRAW_FILES

    success "Profile data merged to: $MERGED_PROFDATA"
}

phase_optimize() {
    info "Phase 3: Building PGO-optimized binary..."

    # Merge profiles first
    phase_merge

    if [ ! -f "$MERGED_PROFDATA" ]; then
        error "Merged profile data not found. Run 'collect' phase first."
    fi

    cd "$PROJECT_ROOT"

    # Clean the release build to ensure full rebuild
    cargo clean --release -p ethrex 2>/dev/null || true

    # Build with PGO data
    RUSTFLAGS="-Cprofile-use=$MERGED_PROFDATA -Cllvm-args=-pgo-warn-missing-function" \
        cargo build --release --bin ethrex

    success "PGO-optimized binary built: target/release/ethrex"

    # Show binary size
    ls -lh "$PROJECT_ROOT/target/release/ethrex"
}

phase_all() {
    info "Running complete PGO build process..."
    echo ""

    phase_instrument
    echo ""

    phase_collect
    echo ""

    phase_optimize
    echo ""

    success "PGO build complete!"
    info "The optimized binary is at: target/release/ethrex"
}

phase_clean() {
    info "Cleaning PGO artifacts..."
    rm -rf "$PGO_DIR"
    success "PGO artifacts removed"
}

show_help() {
    cat << EOF
Profile-Guided Optimization (PGO) Build Script for ethrex

Usage: $0 [phase]

Phases:
  instrument  Build instrumented binary (phase 1)
  collect     Run workload to collect profile data (phase 2)
  optimize    Build optimized binary using profile data (phase 3)
  all         Run all phases sequentially (default)
  clean       Remove PGO artifacts
  help        Show this help message

Example workflow:
  # Full automatic PGO build
  ./scripts/pgo-build.sh all

  # Or manual step-by-step:
  ./scripts/pgo-build.sh instrument
  # Run your custom workload with target/release/ethrex
  ./scripts/pgo-build.sh optimize

Expected performance improvement: 10-15%
EOF
}

# Main
case "${1:-all}" in
    instrument)
        phase_instrument
        ;;
    collect)
        phase_collect
        ;;
    optimize)
        phase_optimize
        ;;
    merge)
        phase_merge
        ;;
    all)
        phase_all
        ;;
    clean)
        phase_clean
        ;;
    help|--help|-h)
        show_help
        ;;
    *)
        error "Unknown phase: $1. Use 'help' for usage information."
        ;;
esac
