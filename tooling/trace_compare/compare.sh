#!/usr/bin/env bash
# Compare `debug_traceTransaction` output across every EL client in a kurtosis enclave.
#
# Prereqs:
#   - `make localnet` already running (or any kurtosis enclave with at least one EL service)
#   - `kurtosis`, `curl`, `jq` on $PATH
#
# Usage:
#   tooling/trace_compare/compare.sh [--enclave NAME] [--tx 0xHASH] [--out DIR]
#
# Defaults:
#   --enclave  lambdanet                (matches the `make localnet` enclave name)
#   --tx       <first tx of latest block>
#   --out      ./trace-compare-<timestamp>
#
# The script:
#   1. Discovers every `el-*` service in the enclave and its host-mapped RPC port.
#   2. If --tx wasn't given, picks the first tx from the latest block (using the
#      first discovered client's RPC).
#   3. Calls `debug_traceTransaction` against every client and saves each response
#      to `<out>/<service>.json`.
#   4. Prints suggested `diff` commands for every pair.
#
# Why this exists: spot-checking that ethrex's `OpcodeStep` wire shape (and any
# future tracer changes) match the other major clients on the same execution.

set -euo pipefail

ENCLAVE="lambdanet"
TX_HASH=""
OUT_DIR=""

usage() {
    sed -n '2,/^set -euo pipefail/p' "$0" | sed -e 's/^# \{0,1\}//' -e '$d'
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --enclave) ENCLAVE="$2"; shift 2 ;;
        --tx)      TX_HASH="$2"; shift 2 ;;
        --out)     OUT_DIR="$2"; shift 2 ;;
        -h|--help) usage 0 ;;
        *) echo "unknown arg: $1" >&2; usage 1 ;;
    esac
done

if [[ -z "$OUT_DIR" ]]; then
    OUT_DIR="./trace-compare-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUT_DIR"

for cmd in kurtosis curl jq; do
    command -v "$cmd" >/dev/null 2>&1 || { echo "error: '$cmd' not on \$PATH" >&2; exit 1; }
done

if ! kurtosis enclave inspect "$ENCLAVE" >/dev/null 2>&1; then
    echo "error: kurtosis enclave '$ENCLAVE' not found. Did you run \`make localnet\`?" >&2
    exit 1
fi

# Discover EL services. Kurtosis names them `el-N-<client>-<cl>`, e.g. `el-1-geth-lighthouse`.
# In `kurtosis enclave inspect` the service name is column 2 (after the UUID).
# Avoid `mapfile` because macOS still ships bash 3.2.
SERVICES=()
while IFS= read -r svc; do
    [[ -n "$svc" ]] && SERVICES+=("$svc")
done < <(
    kurtosis enclave inspect "$ENCLAVE" 2>/dev/null \
        | awk '$2 ~ /^el-/ {print $2}' \
        | sort -u
)

if [[ ${#SERVICES[@]} -eq 0 ]]; then
    echo "error: no EL services found in enclave '$ENCLAVE'" >&2
    exit 1
fi

# Parallel arrays instead of `declare -A` (associative arrays are bash 4+).
# `RPC_NAMES[i]` is the service name, `RPC_URLS[i]` its rpc/http endpoint.
RPC_NAMES=()
RPC_URLS=()
for svc in "${SERVICES[@]}"; do
    # Try `rpc` first (geth/ethrex/reth), then `http` (besu/nethermind sometimes).
    url=$(kurtosis port print "$ENCLAVE" "$svc" rpc 2>/dev/null || true)
    if [[ -z "$url" ]]; then
        url=$(kurtosis port print "$ENCLAVE" "$svc" http 2>/dev/null || true)
    fi
    if [[ -z "$url" ]]; then
        echo "warn: no rpc/http port found for $svc, skipping" >&2
        continue
    fi
    RPC_NAMES+=("$svc")
    RPC_URLS+=("$url")
    echo "$svc -> $url"
done

if [[ ${#RPC_NAMES[@]} -eq 0 ]]; then
    echo "error: no usable RPC URLs discovered" >&2
    exit 1
fi

# Pick a tx if not specified.
if [[ -z "$TX_HASH" ]]; then
    some_svc="${RPC_NAMES[0]}"
    some_url="${RPC_URLS[0]}"
    TX_HASH=$(
        curl -s "$some_url" -H 'content-type: application/json' \
            -d '{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["latest",false]}' \
        | jq -r '.result.transactions[0] // empty'
    )
    if [[ -z "$TX_HASH" ]]; then
        echo "error: 'latest' has no transactions on $some_svc. Specify --tx <hash>." >&2
        exit 1
    fi
    echo "auto-picked tx: $TX_HASH (from $some_svc)"
fi

echo "tracing $TX_HASH across ${#RPC_NAMES[@]} clients..."
for i in "${!RPC_NAMES[@]}"; do
    svc="${RPC_NAMES[$i]}"
    url="${RPC_URLS[$i]}"
    out="$OUT_DIR/${svc}.json"
    curl -s "$url" -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"debug_traceTransaction\",\"params\":[\"$TX_HASH\",{}]}" \
        > "$out"
    if jq -e '.error' "$out" >/dev/null 2>&1; then
        echo "  $svc -> $out (ERROR: $(jq -r '.error.message' "$out"))"
    else
        n=$(jq -r '.result.structLogs | length // 0' "$out")
        echo "  $svc -> $out ($n structLogs)"
    fi
done

echo ""
echo "saved under: $OUT_DIR"
echo ""

# Print pairwise diff commands. structLogs comparison is the interesting bit;
# wrappers and per-step extras can introduce noise.
echo "compare with (structLogs only):"
for ((i=0; i<${#RPC_NAMES[@]}; i++)); do
    for ((j=i+1; j<${#RPC_NAMES[@]}; j++)); do
        a="${RPC_NAMES[i]}"; b="${RPC_NAMES[j]}"
        echo "  diff <(jq '.result.structLogs' $OUT_DIR/$a.json) <(jq '.result.structLogs' $OUT_DIR/$b.json)"
    done
done
