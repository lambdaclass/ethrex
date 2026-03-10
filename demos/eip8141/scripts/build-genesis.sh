#!/usr/bin/env bash
# Build genesis.json starting from the L1 dev genesis and injecting demo contracts.
# Run from the demos/eip8141/ directory.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p contracts/out

echo "Extracting runtime bytecodes..."

# Yul contracts: full binary = constructor + fe + runtime
# GasSponsor: simple constructor (14 bytes = 28 hex chars)
GS_FULL=$(solc --strict-assembly contracts/yul/GasSponsor.yul 2>/dev/null | grep -A1 "Binary representation" | tail -1)
GS_RUNTIME="0x${GS_FULL:28}"

# WebAuthnP256Account: modified constructor reads pubkey args from end of initcode.
# The FULL binary is the initcode (constructor + runtime) used by the factory.
WA_FULL=$(solc --strict-assembly contracts/yul/WebAuthnP256Account.yul 2>/dev/null | grep -A1 "Binary representation" | tail -1)
WA_INITCODE="0x${WA_FULL}"

# Write the initcode to a file for the backend to read at startup
echo -n "$WA_INITCODE" > contracts/out/WebAuthnP256Account.initcode.hex
echo "  WebAuthnP256Account initcode written to contracts/out/WebAuthnP256Account.initcode.hex"

# Solidity contracts: --bin-runtime gives runtime directly
ME_RUNTIME="0x$(solc --via-ir --bin-runtime --optimize --optimize-runs 200 \
  --base-path . @solady/=contracts/deps/solady/ \
  contracts/src/MockERC20.sol 2>/dev/null | tail -1)"

WV_RUNTIME="0x$(solc --via-ir --bin-runtime --optimize --optimize-runs 200 \
  --base-path . @solady/=contracts/deps/solady/ \
  contracts/src/WebAuthnVerifier.sol 2>/dev/null | tail -1)"

# WebAuthnP256AccountFactory
FACTORY_RUNTIME="0x$(solc --via-ir --bin-runtime --optimize --optimize-runs 200 \
  --base-path . @solady/=contracts/deps/solady/ \
  contracts/src/WebAuthnP256AccountFactory.sol 2>/dev/null | tail -1)"

echo "  GasSponsor:                    $(( (${#GS_RUNTIME} - 2) / 2 )) bytes"
echo "  WebAuthnP256Account initcode:  $(( (${#WA_INITCODE} - 2) / 2 )) bytes"
echo "  MockERC20:                     $(( (${#ME_RUNTIME} - 2) / 2 )) bytes"
echo "  WebAuthnVerifier:              $(( (${#WV_RUNTIME} - 2) / 2 )) bytes"
echo "  Factory:                       $(( (${#FACTORY_RUNTIME} - 2) / 2 )) bytes"

# Storage slot for MockERC20 balances (mapping slot 0):
# Dev account: Hardhat #0 (0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
DEV_BALANCE_SLOT="0x723077b8a1b173adc35e5f0e7e3662fd1208212cb629f9c128551ea7168da722"

# 1M tokens (1_000_000 * 10^18 = 0xd3c21bcecceda1000000)
TOKEN_BALANCE="0x00000000000000000000000000000000000000000000d3c21bcecceda1000000"

echo "Injecting demo contracts into L1 dev genesis..."

node -e "
const fs = require('fs');
const genesis = JSON.parse(fs.readFileSync('../../fixtures/genesis/l1.json', 'utf8'));

// Change chain ID for the demo
genesis.config.chainId = 1729;

// Ensure Hardhat #0 dev account is funded
genesis.alloc['0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266'] = {
  balance: '0x200000000000000000000000000000000000000000000000000000000000'
};

// GasSponsor
genesis.alloc['0x1000000000000000000000000000000000000001'] = {
  balance: '0x56BC75E2D63100000',
  code: $(printf '%s' "$GS_RUNTIME" | node -e "process.stdout.write(JSON.stringify(require('fs').readFileSync('/dev/stdin','utf8')))"),
  storage: {
    '0x0000000000000000000000000000000000000000000000000000000000000000':
      '0x0000000000000000000000001000000000000000000000000000000000000002'
  }
};

// MockERC20 (only pre-mint to dev account; user accounts get minted during registration)
genesis.alloc['0x1000000000000000000000000000000000000002'] = {
  balance: '0x0',
  code: $(printf '%s' "$ME_RUNTIME" | node -e "process.stdout.write(JSON.stringify(require('fs').readFileSync('/dev/stdin','utf8')))"),
  storage: {
    '$DEV_BALANCE_SLOT': '$TOKEN_BALANCE'
  }
};

// WebAuthnVerifier
genesis.alloc['0x1000000000000000000000000000000000000004'] = {
  balance: '0x0',
  code: $(printf '%s' "$WV_RUNTIME" | node -e "process.stdout.write(JSON.stringify(require('fs').readFileSync('/dev/stdin','utf8')))"),
  storage: {}
};

// WebAuthnP256AccountFactory (initialized by backend at startup with the account initcode)
genesis.alloc['0x1000000000000000000000000000000000000005'] = {
  balance: '0x0',
  code: $(printf '%s' "$FACTORY_RUNTIME" | node -e "process.stdout.write(JSON.stringify(require('fs').readFileSync('/dev/stdin','utf8')))"),
  storage: {}
};

fs.writeFileSync('genesis.json', JSON.stringify(genesis, null, 2));
console.log('genesis.json written (' + fs.statSync('genesis.json').size + ' bytes)');
"
