#!/bin/bash
set -e

# curl is not in the base ubuntu image
echo "[node-b] Installing curl..."
apt-get update -qq && apt-get install -y -qq --no-install-recommends curl

echo "[node-b] Waiting for node-a HTTP..."
until curl -s --connect-timeout 1 http://node-a:8545 > /dev/null 2>&1; do
    sleep 1
done

echo "[node-b] Fetching node-a enode URL..."
RESPONSE=$(curl -s http://node-a:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')

echo "[node-b] admin_nodeInfo response: $RESPONSE"

ENODE=$(echo "$RESPONSE" | grep -o '"enode":"[^"]*"' | sed 's/"enode":"//;s/"//')

if [ -z "$ENODE" ]; then
    echo "[node-b] ERROR: could not parse enode from response: $RESPONSE"
    exit 1
fi

echo "[node-b] Connecting to: $ENODE"

exec /usr/local/bin/ethrex \
    --network /genesis.json \
    --datadir memory \
    --p2p.addr :: \
    --p2p.port 30303 \
    --nat.extip fd12:3456::3 \
    --http.addr 0.0.0.0 \
    --http.port 8545 \
    --bootnodes "$ENODE"
