#!/usr/bin/env bash
set -euo pipefail

PGO_DIR="${PGO_DIR:-./pgo-data}"
RAW_DIR="${PGO_DIR}/raw"
DATA_DIR="${PGO_DIR}/datadir"
PROFDATA="${PGO_DIR}/merged.profdata"
SYNC_DURATION="${SYNC_DURATION:-15m}"

echo "Warning: This should be run when lighthouse is running, to profile a hoodi sync to be used in PGO."

rm -rf "$DATA_DIR"
mkdir -p "$RAW_DIR"

SYSROOT="$(rustc --print sysroot)"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
LLVM_PROFDATA="${LLVM_PROFDATA:-${SYSROOT}/lib/rustlib/${HOST}/bin/llvm-profdata}"

if [ ! -x "$LLVM_PROFDATA" ]; then
  if command -v llvm-profdata >/dev/null 2>&1; then
    LLVM_PROFDATA="llvm-profdata"
  else
    echo "llvm-profdata not found; install with: rustup component add llvm-tools-preview" >&2
    exit 1
  fi
fi

RUSTFLAGS="-Cprofile-generate=${RAW_DIR} -Cllvm-args=-pgo-warn-missing-function" \
  cargo build --profile pgo-gen -p ethrex

./target/pgo-gen/ethrex --network hoodi --syncmode snap --datadir="$DATA_DIR" \
  --http.addr 0.0.0.0 --metrics --metrics.port 3701 &
PID=$!
( sleep "$SYNC_DURATION"; kill -TERM "$PID" 2>/dev/null || true ) &
wait "$PID" || true

"$LLVM_PROFDATA" merge -o "$PROFDATA" "$RAW_DIR"/*.profraw

RUSTFLAGS="-Cprofile-use=${PROFDATA} -Cllvm-args=-pgo-warn-missing-function" \
  cargo build --profile pgo-use -p ethrex
