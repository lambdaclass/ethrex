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
#   ./tooling/import_benchmark/benchmark.sh mainnet 5
#
set -euo pipefail

NETWORK=$1
NUM_REPETITIONS=$2

touch bench-0.log
bench_id=$(ls bench-*.log | cut -d '-' -f 2 | cut -d '.' -f 1 | sort -n | tail -1)

cd ../..

for i in $(seq 1 $NUM_REPETITIONS); do
    bench_id=$((bench_id + 1))
    rm -rf ~/.local/share/temp
    cp -r ~/.local/share/ethrex_${NETWORK}_bench/ethrex ~/.local/share/temp
    cargo run --release -- --network $NETWORK --datadir ~/.local/share/temp import-bench ~/.local/share/ethrex_${NETWORK}_bench/chain.rlp | tee bench-${bench_id}.log
done
