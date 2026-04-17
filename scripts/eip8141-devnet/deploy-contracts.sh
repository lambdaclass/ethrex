#!/bin/bash
# Deploy EIP-8141 devnet contracts using rex
set -euo pipefail

RPC_URL="${1:?Usage: deploy-contracts.sh <rpc-url> <deployer-private-key>}"
PRIVATE_KEY="${2:?Usage: deploy-contracts.sh <rpc-url> <deployer-private-key>}"
DEPLOYER_ADDR=$(rex address --private-key "$PRIVATE_KEY")

echo "=== Deploying CanonicalPaymaster ==="
echo "RPC: $RPC_URL"
echo "Deployer: $DEPLOYER_ADDR"

# Check deployer balance
BALANCE=$(rex balance "$DEPLOYER_ADDR" --rpc-url "$RPC_URL")
echo "Deployer balance: $BALANCE"

# Fetch the CanonicalPaymaster source
TMPDIR=$(mktemp -d)
curl -sL https://raw.githubusercontent.com/ethereum/EIPs/master/assets/eip-8141/CanonicalPaymaster.sol \
  -o "$TMPDIR/CanonicalPaymaster.sol"
echo "Downloaded CanonicalPaymaster.sol to $TMPDIR"

# Deploy with deployer as the owner/signer (constructor arg)
# Also send 10 ETH during deployment for initial gas sponsorship funds
echo "Deploying..."
PAYMASTER_ADDR=$(rex deploy \
  --contract-path "$TMPDIR/CanonicalPaymaster.sol" \
  --constructor-args "$DEPLOYER_ADDR" \
  --private-key "$PRIVATE_KEY" \
  --rpc-url "$RPC_URL" \
  --value 10ether \
  --print-address)

echo "CanonicalPaymaster deployed at: $PAYMASTER_ADDR"

# Verify deployment
CODE=$(rex code "$PAYMASTER_ADDR" --rpc-url "$RPC_URL")
if [ "$CODE" = "0x" ] || [ -z "$CODE" ]; then
  echo "ERROR: No code at deployed address!"
  rm -rf "$TMPDIR"
  exit 1
fi
echo "Verified: contract has code"

# Fund the paymaster with additional ETH for gas sponsorship
echo "Funding paymaster with 100 ETH..."
rex transfer \
  --to "$PAYMASTER_ADDR" \
  --value 100ether \
  --private-key "$PRIVATE_KEY" \
  --rpc-url "$RPC_URL"

PAYMASTER_BALANCE=$(rex balance "$PAYMASTER_ADDR" --rpc-url "$RPC_URL")
echo "Paymaster balance: $PAYMASTER_BALANCE"

rm -rf "$TMPDIR"

echo ""
echo "=== Deployment Complete ==="
echo "CanonicalPaymaster: $PAYMASTER_ADDR"
echo "Owner/Signer:       $DEPLOYER_ADDR"
