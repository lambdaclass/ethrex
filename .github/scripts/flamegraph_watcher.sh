#!/bin/bash

# Usage:
# ./flamegraph_watcher.sh
# Requires a PROGRAM variable to be set (e.g. ethrex). This $PROGRAM will be killed when the 
# load test finishes.

# This script runs a load test and then kills the node under test. The load test sends a 
# transaction from each rich account to a random one, so we can check their nonce to
# determine that the load test finished.

# TODO: Move this to a cached build outside.
cargo build --release --manifest-path ./cmd/load_test/Cargo.toml

echo "Starting load test"
start_time=$(date +%s)
./target/release/load_test -k ./test_data/private_keys.txt -t eth-transfers -N 1000 -n http://localhost:1729 -w 5 >/dev/null
end_time=$(date +%s)

elapsed=$((end_time - start_time))
minutes=$((elapsed / 60))
seconds=$((elapsed % 60))
echo "All load test transactions included in $minutes min $seconds s, killing node process."

echo killing "$PROGRAM"
sudo pkill "$PROGRAM"

while pgrep -l "$PROGRAM" >/dev/null; do
    echo "$PROGRAM still alive, waiting for it to exit..."
    sleep 10
done
echo "$PROGRAM exited"

# We need this for the following job, to add to the static page
echo "time=$minutes minutes $seconds seconds" >>"$GITHUB_OUTPUT"
