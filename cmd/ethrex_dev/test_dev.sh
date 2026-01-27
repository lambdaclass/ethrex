#!/bin/bash
# Test script for ethrex-dev
# Usage: ./test_dev.sh

set -e

echo "Building ethrex-dev..."
cargo build --package ethrex-dev-bin --quiet

echo "Starting ethrex-dev..."
cargo run --package ethrex-dev-bin --quiet &
PID=$!
sleep 3

cleanup() {
    echo ""
    echo "Stopping server..."
    kill $PID 2>/dev/null || true
    wait $PID 2>/dev/null || true
    exit 0
}
trap cleanup EXIT

echo ""
echo "=== Test 1: Check chain ID ==="
curl -s -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
  http://127.0.0.1:8545
echo ""

echo ""
echo "=== Test 2: Check initial block number ==="
curl -s -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://127.0.0.1:8545
echo ""

echo ""
echo "=== Test 3: Send 1 ETH from account[0] to account[1] ==="
rex send 0x8943545177806ed17b9f23f0a21ee5948ecaa776 \
  -k 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e \
  --value 1000000000000000000 \
  --chain-id 9 \
  --rpc-url http://127.0.0.1:8545 \
  -c -s 2>&1 || true

sleep 1

echo ""
echo "=== Test 4: Check block number after tx ==="
curl -s -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://127.0.0.1:8545
echo ""

echo ""
echo "=== Test 5: Check account[1] balance ==="
curl -s -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_getBalance","params":["0x8943545177806ed17b9f23f0a21ee5948ecaa776","latest"],"id":1}' \
  http://127.0.0.1:8545
echo ""

echo ""
echo "=== All tests completed ==="
