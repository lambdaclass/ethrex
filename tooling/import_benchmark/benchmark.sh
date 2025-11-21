#!/bin/bash

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
    cargo r --release -- --network $NETWORK --datadir ~/.local/share/temp import-bench ~/.local/share/ethrex_${NETWORK}_bench/chain.rlp | tee bench-${bench_id}.log
done
