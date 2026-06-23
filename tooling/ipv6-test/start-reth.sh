#!/bin/bash
set -e

apt-get update -qq && apt-get install -y -qq --no-install-recommends curl

echo "[reth] Waiting for node-a HTTP..."
until curl -s --connect-timeout 1 http://node-a:8545 > /dev/null 2>&1; do
    sleep 1
done

echo "[reth] Fetching node-a enode URL..."
RESPONSE=$(curl -s http://node-a:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')

echo "[reth] admin_nodeInfo response: $RESPONSE"

ENODE=$(echo "$RESPONSE" | grep -o '"enode":"[^"]*"' | sed 's/"enode":"//;s/"//')

if [ -z "$ENODE" ]; then
    echo "[reth] ERROR: could not parse enode from response"
    exit 1
fi

echo "[reth] Connecting to: $ENODE"

exec reth node \
    --chain /genesis.json \
    --datadir /data \
    --addr :: \
    --port 30303 \
    --discovery.addr :: \
    --nat extip:fd12:3456::4 \
    --http --http.addr 0.0.0.0 --http.port 8545 \
    --bootnodes "$ENODE"
