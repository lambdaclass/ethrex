#!/bin/bash
set -e

apt-get update -qq && apt-get install -y -qq --no-install-recommends curl

echo "[besu] Waiting for node-a HTTP..."
until curl -s --connect-timeout 1 http://node-a:8545 > /dev/null 2>&1; do
    sleep 1
done

echo "[besu] Fetching node-a enode URL..."
RESPONSE=$(curl -s http://node-a:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')

echo "[besu] admin_nodeInfo response: $RESPONSE"

ENODE=$(echo "$RESPONSE" | grep -o '"enode":"[^"]*"' | sed 's/"enode":"//;s/"//')

if [ -z "$ENODE" ]; then
    echo "[besu] ERROR: could not parse enode from response"
    exit 1
fi

echo "[besu] Connecting to: $ENODE"

exec /opt/besu/bin/besu \
    --genesis-file=/genesis.json \
    --data-path=/data \
    --p2p-host=fd12:3456::6 \
    --p2p-interface=fd12:3456::6 \
    --nat-method=NONE \
    --p2p-port=30303 \
    --rpc-http-enabled \
    --rpc-http-host=0.0.0.0 \
    --rpc-http-port=8545 \
    --bootnodes="$ENODE"
