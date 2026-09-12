#!/usr/bin/env bash
#
# EIP-8288 aggregation measurements.
#
# Reports proving time, verification time, proof size and peak resident set for the
# leanVM aggregation backend, one dependency count per process so the resident-set
# figure belongs to a single aggregate rather than to a whole test run.
#
# These numbers are specific to a leanVM revision and a machine. What they support
# is documented in docs/eip-8288-measurements.md, which records the environment each
# recorded run was taken on. Re-run this after changing the pinned leanVM revision
# and update that file.
#
# Usage:
#   scripts/eip8288-bench.sh                 # both benchmarks, default counts
#   scripts/eip8288-bench.sh aggregate 1 16  # one benchmark, chosen counts
#   scripts/eip8288-bench.sh absorb 8 32
#
set -euo pipefail

cd "$(dirname "$0")/.."

WHICH="${1:-all}"
[ $# -gt 0 ] && shift || true

case "$WHICH" in
  aggregate) TESTS=(bench_aggregation_cost) ;;
  absorb)    TESTS=(bench_recursive_absorption) ;;
  all)       TESTS=(bench_aggregation_cost bench_recursive_absorption) ;;
  *) echo "usage: $0 [aggregate|absorb|all] [counts...]" >&2; exit 2 ;;
esac

COUNTS=("$@")
[ ${#COUNTS[@]} -eq 0 ] && COUNTS=(1 2 4 8 16 32 64 128)

echo "leanVM revision: $(grep -m1 'rev = ' crates/common/dep-aggregation/Cargo.toml | sed 's/.*rev = "\([^"]*\)".*/\1/')"
echo "ethrex commit:   $(git rev-parse --short HEAD)"
echo "rustc:           $(rustc --version)"
echo

# Build once, then invoke the binary directly so the resident-set reporter measures
# the test rather than cargo.
cargo test -p ethrex-test --features leanvm --release --no-run >/dev/null 2>&1
BIN=$(cargo test -p ethrex-test --features leanvm --release --no-run --message-format=json 2>/dev/null \
      | python3 -c '
import json,sys
for line in sys.stdin:
    try: m = json.loads(line)
    except ValueError: continue
    if m.get("target",{}).get("name") == "ethrex_tests" and m.get("executable"):
        print(m["executable"])
' | tail -1)

if [ -z "${BIN:-}" ]; then
  echo "could not locate the test binary" >&2
  exit 1
fi

for t in "${TESTS[@]}"; do
  for n in "${COUNTS[@]}"; do
    EIP8288_BENCH_DEPS="$n" /usr/bin/time -l "$BIN" \
      --ignored --nocapture --test-threads=1 "$t" 2>&1 \
      | grep -E "EIP8288_BENCH|EIP8288_RECURSE|maximum resident set size" \
      | tr '\n' ' '
    echo
  done
done
