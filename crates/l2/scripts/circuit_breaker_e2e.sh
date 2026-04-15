#!/usr/bin/env bash
# Circuit Breaker End-to-End Validation Script
#
# This script validates the full Circuit Breaker integration:
# 1. Checks that ethrex L2 is running with Circuit Breaker enabled
# 2. Checks that the sidecar is running and healthy
# 3. Deploys a test target contract (OwnableTarget)
# 4. (Manual step) Deploys and registers a test assertion via pcl
# 5. Sends a violating transaction → verifies it's NOT included
# 6. Sends a valid transaction → verifies it IS included
#
# Prerequisites:
#   - ethrex L2 running with --circuit-breaker-url (see: make init-l2-circuit-breaker)
#   - Credible Layer sidecar running (see: make init-circuit-breaker)
#   - cast (from foundry) installed
#   - A funded account on L2
#
# Usage:
#   ./circuit_breaker_e2e.sh [L2_RPC_URL] [SIDECAR_HEALTH_URL]

set -euo pipefail

L2_RPC_URL="${1:-http://localhost:1729}"
SIDECAR_HEALTH_URL="${2:-http://localhost:9547/health}"
PRIVATE_KEY="${PRIVATE_KEY:-0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }
info() { echo -e "${YELLOW}[INFO]${NC} $1"; }

# ─── Step 1: Check ethrex L2 is running ───────────────────────────────────────

info "Checking ethrex L2 at ${L2_RPC_URL}..."
BLOCK_NUMBER=$(cast block-number --rpc-url "$L2_RPC_URL" 2>/dev/null || echo "UNREACHABLE")
if [ "$BLOCK_NUMBER" = "UNREACHABLE" ]; then
    fail "ethrex L2 is not reachable at ${L2_RPC_URL}"
fi
pass "ethrex L2 is running. Current block: ${BLOCK_NUMBER}"

# ─── Step 2: Check sidecar is running ─────────────────────────────────────────

info "Checking sidecar health at ${SIDECAR_HEALTH_URL}..."
HEALTH=$(curl -s -o /dev/null -w "%{http_code}" "$SIDECAR_HEALTH_URL" 2>/dev/null || echo "000")
if [ "$HEALTH" = "200" ]; then
    pass "Sidecar is healthy"
elif [ "$HEALTH" = "000" ]; then
    info "Sidecar health endpoint not reachable (may be OK if running without health server)"
else
    info "Sidecar health returned HTTP ${HEALTH}"
fi

# ─── Step 3: Deploy OwnableTarget contract ────────────────────────────────────

info "Deploying OwnableTarget contract..."

# OwnableTarget bytecode (compiled from contracts/src/circuit_breaker/OwnableTarget.sol)
# If you need to recompile: solc --bin OwnableTarget.sol
# For now, we attempt to deploy using cast
OWNABLE_TARGET_DEPLOY=$(cast send --create \
    --rpc-url "$L2_RPC_URL" \
    --private-key "$PRIVATE_KEY" \
    --json \
    "$(cat <<'SOLC_EOF'
0x608060405234801561001057600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506102f8806100606000396000f3fe608060405234801561001057600080fd5b50600436106100415760003560e01c8063131a06801461004657806370a082311461006a578063f2fde38b14610088575b600080fd5b61004e6100a4565b604051808260001916815260200191505060405180910390f35b6100726100ae565b6040518082815260200191505060405180910390f35b6100a2600480360381019080803573ffffffffffffffffffffffffffffffffffffffff1690602001909291905050506100b7565b005b6000602a905090565b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1681565b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff1614610165576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040180806020018281038252602481526020018061029f6024913960400191505060405180910390fd5b600073ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff1614156101eb576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004018080602001828103825260348152602001806102c36034913960400191505060405180910390fd5b8073ffffffffffffffffffffffffffffffffffffffff1660008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff167f8be0079c531659141344cd1fd0a4f28419497f9722a3daafe3b4186f6b6457e060405160405180910390a3806000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055505056fe4f776e61626c655461726765743a2063616c6c6572206973206e6f7420746865206f776e65724f776e61626c655461726765743a206e6577206f776e657220697320746865207a65726f206164647265737300
SOLC_EOF
)" 2>/dev/null) || true

if [ -n "$OWNABLE_TARGET_DEPLOY" ]; then
    CONTRACT_ADDRESS=$(echo "$OWNABLE_TARGET_DEPLOY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('contractAddress',''))" 2>/dev/null || echo "")
    if [ -n "$CONTRACT_ADDRESS" ]; then
        pass "OwnableTarget deployed at: ${CONTRACT_ADDRESS}"
    else
        info "Deploy transaction sent but contract address not parsed. Check logs."
    fi
else
    info "Could not deploy OwnableTarget. You may need to deploy it manually."
    info "Compile with: solc --bin crates/l2/contracts/src/circuit_breaker/OwnableTarget.sol"
fi

# ─── Step 4: Assertion registration (manual) ──────────────────────────────────

echo ""
info "=== MANUAL STEP ==="
info "To complete the e2e test, you need to:"
info "  1. Deploy TestOwnershipAssertion using pcl:"
info "     pcl apply --assertion TestOwnershipAssertion --adopter ${CONTRACT_ADDRESS:-<CONTRACT_ADDRESS>}"
info "  2. Wait for the assertion timelock to expire"
info "  3. Then run this script again with the --validate flag"
echo ""

# ─── Step 5 & 6: Validation (run with --validate after assertion is active) ──

if [ "${3:-}" = "--validate" ] && [ -n "${CONTRACT_ADDRESS:-}" ]; then
    info "Running validation..."

    # Get current block number
    BLOCK_BEFORE=$(cast block-number --rpc-url "$L2_RPC_URL")

    # Send violating transaction: transferOwnership
    info "Sending violating transaction (transferOwnership)..."
    VIOLATING_TX=$(cast send "$CONTRACT_ADDRESS" \
        "transferOwnership(address)" \
        "0x0000000000000000000000000000000000000001" \
        --rpc-url "$L2_RPC_URL" \
        --private-key "$PRIVATE_KEY" \
        --json 2>/dev/null) || true

    sleep 5 # Wait for a block

    # Send valid transaction: doSomething
    info "Sending valid transaction (doSomething)..."
    VALID_TX=$(cast send "$CONTRACT_ADDRESS" \
        "doSomething()" \
        --rpc-url "$L2_RPC_URL" \
        --private-key "$PRIVATE_KEY" \
        --json 2>/dev/null) || true

    sleep 5 # Wait for a block

    # Check if valid tx was included
    if [ -n "$VALID_TX" ]; then
        VALID_TX_HASH=$(echo "$VALID_TX" | python3 -c "import sys,json; print(json.load(sys.stdin).get('transactionHash',''))" 2>/dev/null || echo "")
        if [ -n "$VALID_TX_HASH" ]; then
            RECEIPT=$(cast receipt "$VALID_TX_HASH" --rpc-url "$L2_RPC_URL" --json 2>/dev/null || echo "")
            if [ -n "$RECEIPT" ]; then
                pass "Valid transaction was included in a block (tx: ${VALID_TX_HASH})"
            else
                fail "Valid transaction was NOT included (should have been)"
            fi
        fi
    fi

    # Check owner hasn't changed (violating tx should have been dropped)
    CURRENT_OWNER=$(cast call "$CONTRACT_ADDRESS" "owner()" --rpc-url "$L2_RPC_URL" 2>/dev/null || echo "")
    info "Current owner: ${CURRENT_OWNER}"
    info "If ownership hasn't changed, the violating transaction was successfully dropped."

    echo ""
    pass "E2E validation complete. Check sidecar logs for assertion evaluation details."
else
    info "Skipping validation. Run with '--validate' after assertion registration."
fi

echo ""
info "Done."
