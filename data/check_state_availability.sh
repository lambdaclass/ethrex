#!/bin/bash

# Check if rpcs.txt exists
if [ ! -f rpcs.txt ]; then
    echo "Error: rpcs.txt not found."
    exit 1
fi

echo "reading file"

# Select a random RPC URL from rpcs.txt
random_rpc=$(shuf -n 1 rpcs.txt)

echo "checking balance"

balance=$(rex l2 balance 0xe25583099ba105d9ec0a67f5ae86d90e50036425 $random_rpc 2>&1)

if [[ $balance == "0" ]]; then
    echo "Error: Balance is zero in 0xe25583099ba105d9ec0a67f5ae86d90e50036425. Cannot proceed with the transaction."
    exit 1
fi

echo "making transfer"

echo $(rex l2 transfer 100000000 0x00097b4463159340ac83b9bdf657c304cd70c11c 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d $random_rpc --cast --silent 2>&1)

echo "taking tx_hash"

# Send the transaction and capture the output
tx_output=$(rex l2 transfer 100000000 0x00097b4463159340ac83b9bdf657c304cd70c11c 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d $random_rpc --cast --silent 2>&1)

# Extract the transaction hash (assuming it is the last line of the output)
tx_hash=$(echo "$tx_output" | tail -n 1)

# Validate that the transaction hash is a valid Ethereum hash (0x followed by 64 hex characters)
if [[ ! $tx_hash =~ ^0x[0-9a-fA-F]{64}$ ]]; then
    echo "Error: Invalid transaction hash obtained: $tx_hash"
    exit 1
fi

# Get total number of RPC URLs
total=$(wc -l < rpcs.txt | tr -d '[:space:]')

# Handle case where there are no URLs
if [ $total -eq 0 ]; then
    echo "0/0"
    exit 0
fi

# Run receipt queries in parallel and capture success/failure
output=$(cat rpcs.txt | xargs -I {} -P 10 bash -c 'rex l2 receipt '"$tx_hash"' {} > /dev/null 2>&1 && echo "success" || echo "failure"')

# Count successful calls
successful=$(echo "$output" | grep -c "success")

while [[ $successful -lt $total ]]; do
    output=$(cat rpcs.txt | xargs -I {} -P 10 bash -c 'rex l2 receipt '"$tx_hash"' {} > /dev/null 2>&1 && echo "success" || echo "failure"')
    successful=$(echo "$output" | grep -c "success")
    echo "$successful/$total $(date +%S,%N)"
done

