#!/bin/bash

parse_args() {
    if [ $# -ne 3 ]; then
        echo "Usage: $0 <number_of_nodes> <private_keys_file> <bootnode_enode>"
        exit 1
    fi

    n=$1
    private_keys_file=$2
    bootnode=$3
}

read_private_keys() {
    if [ ! -f "$private_keys_file" ]; then
        echo "Error: $private_keys_file not found. Please provide a valid path to the keys file."
        exit 1
    fi

    keys=()
    while IFS= read -r line; do
        keys+=("$line")

    done < "$private_keys_file"
    
    # The first two keys are reserved.
    if [ $n -gt 0 ] && [ ${#keys[@]} -lt $((2*n + 2)) ]; then
        echo "Error: Not enough keys in $private_keys_file. Need at least $((2*n + 2)) keys."
        exit 1
    fi
}

datadirs_path="data/datadirs.txt"
pids_path="data/pids.txt"
rpcs_path="data/rpcs.txt"
logs_path="data/logs"

setup() {
    mkdir -p data
    mkdir -p $logs_path
    > $datadirs_path
    > $pids_path
    > $rpcs_path
}

run_nodes() {
    proof_coord_base=4566
    http_base=1729
    p2p_base=30303

    genesis_path="test_data/genesis-l2.json"
    tmux_session=$(tmux display-message -p '#S')

    for i in $(seq 1 $n); do
        committer_key=${keys[$((2*i))]}
        proof_coord_key=${keys[$((2*i+1))]}

        proof_coord_port=$((proof_coord_base + i))
        http_port=$((http_base + i))
        p2p_port=$((p2p_base + i))
        discovery_port=$p2p_port
        datadir="ethrex_l2_$i"

        cmd_str="cargo run --release --bin ethrex --features l2 -- l2 init \\
        --watcher.block-delay 0 \\
        --eth.rpc-url http://localhost:8545 \\
        --block-producer.coinbase-address 0xacb3bb54d7c5295c158184044bdeedd9aa426607 \\
        --committer.l1-private-key \"$committer_key\" \\
        --proof-coordinator.l1-private-key \"$proof_coord_key\" \\
        --network $genesis_path \\
        --datadir \"$datadir\" \\
        --proof-coordinator.addr 127.0.0.1 --proof-coordinator.port \"$proof_coord_port\" \\
        --http.port \"$http_port\" \\
        --state-updater.sequencer-registry \"$sequencer_registry_address\" \\
        --l1.on-chain-proposer-address \"$on_chain_proposer_address\" \\
        --l1.bridge-address \"$bridge_address\" \\
        --based \\
        --p2p.enabled --p2p.port \"$p2p_port\" --discovery.port \"$discovery_port\" --bootnodes \"$bootnode\""

        echo $datadir >> $datadirs_path
        echo "http://localhost:$http_port" >> $rpcs_path

        tmux_cmd="echo 'Starting node $i with committer key: $committer_key and proof coordinator key: $proof_coord_key'; "
        tmux_cmd+="$cmd_str & echo \$! > \"$pid_file\"; wait %1; "
        tmux_cmd+="echo 'Node $i completed with exit code \$?'; read"

        tmux new-window -t $tmux_session: -n "ethrex-node-$i" "$tmux_cmd"
        
        sleep 0.5
        
        if [ -f "$pid_file" ]; then
            actual_pid=$(cat "$pid_file")
            echo "$actual_pid" >> $pids_path
        else
            echo "Warning: Could not capture PID for node $i - PID file not created"
            echo "unknown-$i" >> $pids_path
        fi
    done
}

export $(cat crates/l2/.env | xargs)
sequencer_registry_address=$ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS
on_chain_proposer_address=$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS
bridge_address=$ETHREX_WATCHER_BRIDGE_ADDRESS


#echo "$ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS"

parse_args "$@"
read_private_keys
setup
run_nodes
