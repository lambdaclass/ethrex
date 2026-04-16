#!/bin/sh
set -e

# Geth image is Alpine-based
apk add --no-cache curl

echo "[geth] Initializing genesis..."
geth init --datadir /data /genesis.json

echo "[geth] Waiting for node-a HTTP..."
until curl -s --connect-timeout 1 http://node-a:8545 > /dev/null 2>&1; do
    sleep 1
done

echo "[geth] Fetching node-a enode URL..."
RESPONSE=$(curl -s http://node-a:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')

echo "[geth] admin_nodeInfo response: $RESPONSE"

ENODE=$(echo "$RESPONSE" | grep -o '"enode":"[^"]*"' | sed 's/"enode":"//;s/"//')

if [ -z "$ENODE" ]; then
    echo "[geth] ERROR: could not parse enode from response"
    exit 1
fi

echo "[geth] Connecting to: $ENODE"

exec geth \
    --datadir /data \
    --port 30303 \
    --nat extip:172.28.0.5 \
    --http --http.addr 0.0.0.0 --http.port 8545 \
    --bootnodes "$ENODE" \
    --verbosity 4 \
    --nodiscover=false
