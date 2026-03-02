#!/bin/bash
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Running GEVM benchmarks ==="
(cd "$DIR" && go test -bench=. -benchmem -count=5 -timeout=600s . 2>&1 | tee gevm.txt)

echo ""
echo "=== Running go-ethereum benchmarks ==="
(cd "$DIR/gethbench" && go test -bench=. -benchmem -count=5 -timeout=600s . 2>&1 | tee ../geth.txt)

echo ""
echo "=== Comparison ==="
if command -v benchstat &>/dev/null; then
	(cd "$DIR" && benchstat gevm.txt geth.txt)
else
	echo "benchstat not found. Install with:"
	echo "  go install golang.org/x/perf/cmd/benchstat@latest"
	echo ""
	echo "Results saved to $DIR/gevm.txt and $DIR/geth.txt"
fi
