#!/bin/bash
# Deploy EIP-8141 devnet contracts using rex and solc.
#
# Prerequisites: rex, solc (>= 0.8.28)
# Usage: ./deploy-contracts.sh <rpc-url> <deployer-private-key>
#
# Deploys:
#   1. MockToken   — returns 1 for any balanceOf() call
#   2. GasSponsor  — compiled from GasSponsor.yul, funded with 100 ETH, configured with MockToken

set -euo pipefail

RPC_URL="${1:?Usage: deploy-contracts.sh <rpc-url> <deployer-private-key>}"
PRIVATE_KEY="${2:?Usage: deploy-contracts.sh <rpc-url> <deployer-private-key>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

DEPLOYER_ADDR=$(rex address --private-key "$PRIVATE_KEY")

echo "============================================"
echo "  EIP-8141 Devnet Contract Deployment"
echo "============================================"
echo "RPC:      $RPC_URL"
echo "Deployer: $DEPLOYER_ADDR"
echo "Balance:  $(rex balance "$DEPLOYER_ADDR" --rpc-url "$RPC_URL") wei"
echo ""

# ── Step 1: Deploy MockToken ──────────────────────────────────────────
# Minimal contract that returns 1 (as uint256) for any call.
# Used so GasSponsor.verify() sees a non-zero balanceOf for any sender.
#
# Runtime (8 bytes): PUSH1(1) PUSH0 MSTORE PUSH1(0x20) PUSH0 RETURN
# Initcode (10 bytes): PUSH1(8) PUSH1(10) PUSH0 CODECOPY PUSH1(8) PUSH0 RETURN
echo "=== Step 1: Deploy MockToken ==="
MOCK_RUNTIME="60015f5260205ff3"
MOCK_INITCODE="6008600a5f3960085ff3${MOCK_RUNTIME}"
TOKEN_ADDR=$(rex deploy --bytecode "$MOCK_INITCODE" --private-key "$PRIVATE_KEY" --rpc-url "$RPC_URL" --print-address)
echo "MockToken: $TOKEN_ADDR"

# ── Step 2: Compile and deploy GasSponsor ─────────────────────────────
echo ""
echo "=== Step 2: Compile and deploy GasSponsor ==="
GAS_SPONSOR_YUL="$SCRIPT_DIR/contracts/GasSponsor.yul"
if [ ! -f "$GAS_SPONSOR_YUL" ]; then
    echo "ERROR: GasSponsor.yul not found at $GAS_SPONSOR_YUL"
    exit 1
fi

GS_BYTECODE=$(solc --strict-assembly "$GAS_SPONSOR_YUL" 2>/dev/null | grep -A1 "Binary representation" | tail -1)
if [ -z "$GS_BYTECODE" ]; then
    echo "ERROR: solc compilation failed"
    exit 1
fi

# Verify APPROVE has scope=2 (not the solc optimization bug with scope=1)
# Pattern: PUSH1(2) [optional DUP1] PUSH0 PUSH0 APPROVE
echo "$GS_BYTECODE" | python3 -c "
import sys
code = bytes.fromhex(sys.stdin.read().strip())
for i, b in enumerate(code):
    if b != 0xAA: continue
    # Walk backwards past PUSH0s and DUP1s to find the PUSH1 scope value
    j = i - 1
    while j >= 0 and code[j] in (0x5f, 0x80):  # PUSH0=0x5f, DUP1=0x80
        j -= 1
    if j >= 1 and code[j-1] == 0x60:  # PUSH1
        scope = code[j]
        if scope != 2:
            print(f'ERROR: APPROVE scope is {scope}, expected 2. solc optimization bug!')
            sys.exit(1)
        print(f'  APPROVE scope verified: {scope} (payer approval)')
" || exit 1

SPONSOR_ADDR=$(rex deploy \
    --bytecode "$GS_BYTECODE" \
    --private-key "$PRIVATE_KEY" \
    --rpc-url "$RPC_URL" \
    --value 100000000000000000000 \
    --print-address)
echo "GasSponsor: $SPONSOR_ADDR (funded with 100 ETH)"

# ── Step 3: Configure GasSponsor with MockToken ───────────────────────
echo ""
echo "=== Step 3: Configure GasSponsor ==="
TOKEN_CLEAN=$(echo "$TOKEN_ADDR" | sed 's/0x//')
SETCONFIG_DATA="0x20e3dbd4$(printf '%064s' "$TOKEN_CLEAN" | tr ' ' '0')"
rex send "$SPONSOR_ADDR" --calldata "$SETCONFIG_DATA" --private-key "$PRIVATE_KEY" --rpc-url "$RPC_URL" --silent

# Verify
CONFIGURED_TOKEN=$(rex call "$SPONSOR_ADDR" --calldata 0xfc0c546a --rpc-url "$RPC_URL")
echo "Configured token: $CONFIGURED_TOKEN"

# ── Step 4: Compile and deploy CanonicalPaymaster ─────────────────────
echo ""
echo "=== Step 4: Compile and deploy CanonicalPaymaster ==="
CANONICAL_YUL="$SCRIPT_DIR/contracts/CanonicalPaymaster.yul"
if [ ! -f "$CANONICAL_YUL" ]; then
    echo "WARNING: CanonicalPaymaster.yul not found, skipping"
else
    CP_BYTECODE=$(solc --strict-assembly "$CANONICAL_YUL" 2>/dev/null | grep -A1 "Binary representation" | tail -1)
    if [ -z "$CP_BYTECODE" ]; then
        echo "ERROR: CanonicalPaymaster compilation failed"
    else
        # Verify APPROVE scope
        echo "$CP_BYTECODE" | python3 -c "
import sys
code = bytes.fromhex(sys.stdin.read().strip())
for i, b in enumerate(code):
    if b != 0xAA: continue
    j = i - 1
    while j >= 0 and code[j] in (0x5f, 0x80): j -= 1
    if j >= 1 and code[j-1] == 0x60:
        scope = code[j]
        if scope != 2:
            print(f'ERROR: APPROVE scope is {scope}, expected 2')
            sys.exit(1)
        print(f'  APPROVE scope verified: {scope}')
" || exit 1

        CANONICAL_ADDR=$(rex deploy \
            --bytecode "$CP_BYTECODE" \
            --constructor-args "address:$DEPLOYER_ADDR" \
            --private-key "$PRIVATE_KEY" \
            --rpc-url "$RPC_URL" \
            --value 100000000000000000000 \
            --print-address)
        echo "CanonicalPaymaster: $CANONICAL_ADDR (owner=$DEPLOYER_ADDR, funded 100 ETH)"
    fi
fi

echo ""
echo "============================================"
echo "  Deployment Complete"
echo "============================================"
echo "MockToken:          $TOKEN_ADDR"
echo "GasSponsor:         $SPONSOR_ADDR"
echo "  Balance:          $(rex balance "$SPONSOR_ADDR" --rpc-url "$RPC_URL") wei"
echo "  Token:            $CONFIGURED_TOKEN"
if [ -n "${CANONICAL_ADDR:-}" ]; then
echo "CanonicalPaymaster: $CANONICAL_ADDR"
echo "  Owner:            $DEPLOYER_ADDR"
echo "  Balance:          $(rex balance "$CANONICAL_ADDR" --rpc-url "$RPC_URL") wei"
fi
echo ""
echo "Test commands:"
echo "  # Self-verified frame tx (rex)"
echo "  rex frame send --to 0xC0FFEE --value 0.01ether --private-key <key> --rpc-url $RPC_URL"
echo ""
echo "  # Sponsored with GasSponsor (rex)"
echo "  rex frame send --to 0xC0FFEE --value 0.01ether --sponsor $SPONSOR_ADDR --sponsor-calldata 0xfc735e99 --private-key <key> --rpc-url $RPC_URL"
echo ""
echo "  # Check frame tx receipt (rex)"
echo "  rex frame receipt <TX_HASH> --rpc-url $RPC_URL"
echo ""
if [ -n "${CANONICAL_ADDR:-}" ]; then
echo "  # CanonicalPaymaster (requires Python — owner must co-sign sig_hash)"
echo "  python3 test-canonical-paymaster.py --rpc-url $RPC_URL --sender-key <key> --owner-key <owner-key> --paymaster $CANONICAL_ADDR"
fi
echo "============================================"
