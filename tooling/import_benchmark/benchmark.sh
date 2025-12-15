#!/bin/bash

# tooling/import_benchmark/benchmark.sh
# ----------------------------------
# Usage: tooling/import_benchmark/benchmark.sh <NETWORK> <NUM_REPETITIONS>
#
# Arguments:
#   <NETWORK>         Name of the network to use (e.g.: 'mainnet', 'hoodi', 'sepolia').
#   <NUM_REPETITIONS> Number of times to run the benchmark (positive integer).
#
# Description:
#   This script runs the import benchmark repeatedly and saves each run's output
#   to an incrementally numbered log file named `bench-<id>.log`.
#
# Example:
#   ./tooling/import_benchmark/benchmark.sh mainnet 5     # run 5 repetitions with mainnet
#   ./tooling/import_benchmark/benchmark.sh mainnet 5 10  # same, but start with bench_id 10
#
set -euo pipefail

NETWORK=$1
NUM_REPETITIONS=$2
# Optional third argument: starting bench id. If provided, this value is used
# as the current `bench_id` (the script will increment it before the first run).
START_BENCH_ID=${3:-}

# Get the directory of this script, so it can be ran from anywhere
BENCHMARKS_DIR=$(
    cd "$(dirname "${BASH_SOURCE[0]}")"
    pwd -P
)

# Go to ethrex/tooling/import_benchmark
cd "$BENCHMARKS_DIR"

# Create directory for benchmark results, if it doesn't exist
mkdir -p bench_results

if [ -n "$START_BENCH_ID" ]; then
    if ! [[ "$START_BENCH_ID" =~ ^[0-9]+$ ]]; then
        echo "Error: START_BENCH_ID must be a non-negative integer" >&2
        exit 2
    fi
    bench_id=$START_BENCH_ID
else
    # Determine the next available bench id
    cd bench_results
    touch bench-0.log
    bench_id=$(ls bench-*.log | cut -d '-' -f 2 | cut -d '.' -f 1 | sort -n | tail -1)
    # When no START_BENCH_ID is supplied, start from the next available id
    bench_id=$((bench_id + 1))
    cd ..
fi

# Move to repo root
cd ../..

for i in $(seq 1 $NUM_REPETITIONS); do
    # Remove previous temp data dir and copy base state to it
    rm -rf ~/.local/share/temp
    cp -r ~/.local/share/ethrex_${NETWORK}_bench/ethrex ~/.local/share/temp

    # Run the benchmark and save output to log file in the benchmarks directory
    cargo run --release -- --network $NETWORK --datadir ~/.local/share/temp import-bench ~/.local/share/ethrex_${NETWORK}_bench/chain.rlp | tee "${BENCHMARKS_DIR}/bench_results/bench-${bench_id}.log"

    bench_id=$((bench_id + 1))
done
