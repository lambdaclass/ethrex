#!/usr/bin/env bash
# verify-contracts.sh — Verify EIP-8141 demo contracts on Blockscout.
#
# Verifies MockERC20 and WebAuthnVerifier via the Blockscout v1 API using
# standard-json-input format (required for --via-ir compilation).
#
# GasSponsor and WebAuthnP256Account are compiled from Yul with custom
# opcodes (verbatim) and cannot be verified via Blockscout's standard
# Solidity verifier.
#
# Usage:
#   BLOCKSCOUT_URL=http://localhost:8082 ./scripts/verify-contracts.sh
#   ./scripts/verify-contracts.sh  # defaults to http://localhost:8082

set -euo pipefail

BLOCKSCOUT_URL="${BLOCKSCOUT_URL:-http://localhost:8082}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CONTRACTS_DIR="$SCRIPT_DIR/../contracts"

echo "Blockscout URL: $BLOCKSCOUT_URL"
echo "Contracts dir:  $CONTRACTS_DIR"

# ── Detect compiler version from deployed bytecode metadata ────────

echo ""
echo "Detecting compiler version from on-chain bytecode metadata..."
COMPILER_VERSION=$(python3 -c "
import json, urllib.request

# Get deployed bytecode of MockERC20
req = urllib.request.Request('${BLOCKSCOUT_URL}/api/v2/smart-contracts/verification/config')
resp = urllib.request.urlopen(req)
data = json.loads(resp.read())
versions = data.get('solidity_compiler_versions', data.get('solidity_versions', []))

# Find the installed solc version
import subprocess
try:
    result = subprocess.run(['solc', '--version'], capture_output=True, text=True)
    # Extract version like '0.8.31+commit.fd3a2265' from output
    for line in result.stdout.split('\n'):
        if 'Version:' in line:
            ver = line.split('Version:')[1].strip().split('.Linux')[0].split('.Darwin')[0]
            target = f'v{ver}'
            break
    matches = [v for v in versions if target in v]
    if matches:
        print(matches[0])
    else:
        # Fall back to newest 0.8.x
        matches = [v for v in versions if 'v0.8.' in v]
        print(matches[0] if matches else 'NOT_FOUND')
except Exception:
    matches = [v for v in versions if 'v0.8.' in v]
    print(matches[0] if matches else 'NOT_FOUND')
")

if [ "$COMPILER_VERSION" = "NOT_FOUND" ]; then
    echo "ERROR: Could not determine compiler version"
    exit 1
fi

echo "Using compiler: $COMPILER_VERSION"

# ── Verification via v1 API with standard-json-input ───────────────

python3 "$SCRIPT_DIR/verify-contracts.py" \
    "$BLOCKSCOUT_URL" \
    "$COMPILER_VERSION" \
    "$CONTRACTS_DIR"
