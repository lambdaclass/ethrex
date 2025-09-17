#!/bin/bash

HIVE_CMD="./hive"
CLIENT_FILE=".github/config/hive/clients.yaml"
CLIENT="ethrex"
SIM="ethereum/engine"
SIM_LIMIT="engine-cancun/Invalid Missing Ancestor Syncing ReOrg, Transaction Nonce, EmptyTxs=False, CanonicalReOrg=True, Invalid P9 (Cancun)"

for ((i=1; i<=1000; i++)); do
    echo "Running hive command..."
    $HIVE_CMD \
        --client-file "$CLIENT_FILE" \
        --client "$CLIENT" \
        --sim "$SIM" \
        --sim.limit "$SIM_LIMIT" \
        --sim.parallelism 16 \
        --sim.loglevel 1 \
        --docker.output

    echo "[$i/1000] Command completed at $(date)"
done
