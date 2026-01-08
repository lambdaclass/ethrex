#!/usr/bin/env bash
set -euo pipefail

PGO_DIR="${PGO_DIR:-./pgo-data}"
RAW_DIR="${PGO_DIR}/raw"
DATA_DIR="${PGO_DIR}/datadir"
PROFDATA="${PGO_DIR}/merged.profdata"
PROFDATA_ABS="$(cd "$(dirname "$PROFDATA")" && pwd)/$(basename "$PROFDATA")"
SYNC_DURATION="${SYNC_DURATION:-5m}"
LIGHTHOUSE_DATADIR="${LIGHTHOUSE_DATADIR:-${PGO_DIR}/lighthouse}"
LIGHTHOUSE_JWT="${LIGHTHOUSE_JWT:-./jwt.hex}"

echo "Info: Starting lighthouse and ethrex to profile a Hoodi sync for PGO."

rm -rf "$DATA_DIR"
mkdir -p "$RAW_DIR"
mkdir -p "$LIGHTHOUSE_DATADIR"

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

lighthouse bn --network hoodi --datadir "$LIGHTHOUSE_DATADIR" \
  --execution-endpoint http://localhost:8551 \
  --execution-jwt "$LIGHTHOUSE_JWT" \
  --http \
  --checkpoint-sync-url https://hoodi-checkpoint-sync.stakely.io/ \
  --http-address 0.0.0.0 \
  --purge-db-force &
LIGHTHOUSE_PID=$!

./target/pgo-gen/ethrex --network hoodi --syncmode snap --datadir="$DATA_DIR" \
  --http.addr 0.0.0.0 --metrics --metrics.port 3701 &
ETHREX_PID=$!
( sleep "$SYNC_DURATION"; kill -TERM "$ETHREX_PID" "$LIGHTHOUSE_PID" 2>/dev/null || true ) &
wait "$ETHREX_PID" || true
wait "$LIGHTHOUSE_PID" || true

"$LLVM_PROFDATA" merge -o "$PROFDATA" "$RAW_DIR"/*.profraw

RUSTFLAGS="-Cprofile-use=${PROFDATA_ABS}" \
  cargo build --profile pgo-use -p ethrex
