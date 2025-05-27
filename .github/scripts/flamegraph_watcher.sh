#!/bin/bash
set -e

# This script runs a load test and then kills the node under test. The load test sends a
# transaction from each rich account to a random one, so we can check their nonce to
# determine that the load test finished.
#
# Usage:
# ./flamegraph_watcher.sh
# Requires a PROGRAM variable to be set (e.g. ethrex). This $PROGRAM will be killed when the
# load test finishes. Must be run from the context of the repo root.

# TODO(#2486): Move this to a cached build outside.
echo "Building load test"
cargo build --release --manifest-path ./tooling/load_test/Cargo.toml

echo "Starting load test"
start_time=$(date +%s)
RUST_BACKTRACE=1 ./target/release/load_test -k ./test_data/private_keys.txt -t eth-transfers -N 1000 -n http://localhost:1729 -w 1
end_time=$(date +%s)

elapsed=$((end_time - start_time))
minutes=$((elapsed / 60))
seconds=$((elapsed % 60))
echo "All load test transactions included in $minutes min $seconds s, killing node process."

echo killing "$PROGRAM"
sudo pkill "$PROGRAM"

while pgrep -l "perf" >/dev/null; do
    echo "perf still alive, waiting for it to exit..."
    sleep 10
done
echo "perf exited"

# We need this for the following job, to add to the static page
echo "time=$minutes minutes $seconds seconds" >>"$GITHUB_OUTPUT"
