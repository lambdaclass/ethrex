#!/usr/bin/env bash
# gen_structlog_fixtures.sh — Reference procedure for EIP-3155 struct-log fixtures.
#
# *** IMPORTANT: FORMAT DIVERGENCE ***
# ethrex emits STRICT EIP-3155 format (https://eips.ethereum.org/EIPS/eip-3155).
# geth's `evm` CLI and `debug_traceTransaction` emit `structLogLegacy` format
# (toLegacyJSON in eth/tracers/logger/logger.go), which differs in several ways:
#
#   - gas/gasCost: geth emits decimal numbers; EIP-3155 requires hex strings ("0x...")
#   - op: geth emits the opcode NAME string; EIP-3155 requires the byte VALUE (decimal)
#   - opName: EIP-3155 adds this always-present string field; geth's legacy format omits it
#   - memSize: EIP-3155 requires this always-present decimal field; geth's legacy omits it
#   - refund: EIP-3155 always emits this as hex string "0x0"; geth omits it when zero
#   - returnData: EIP-3155 always emits this as "0x"; geth omits it when absent/empty
#   - stack: EIP-3155 emits null when disabled; geth omits the field
#   - result wrapper: EIP-3155 uses {pass, gasUsed, output}; geth uses {gas, failed, returnValue}
#
# Running geth's evm CLI or `debug_traceTransaction` will NOT produce byte-identical
# output to ethrex's EIP-3155 tracer. This script is kept as a debugging aid to
# understand gas values and opcode sequences, but it cannot serve as a reference
# for fixture regeneration.
#
# To regenerate the ethrex fixtures, use the ignored helper tests in
# test/tests/levm/struct_log_fixture_gen.rs:
#
#   cargo test -p ethrex-levm-test print_ -- --nocapture --ignored
#
# Then copy each printed JSON block to the corresponding fixture file.
#
# This script documents the original geth-based procedure for historical reference.
#
# PINNED GETH VERSION
# ===================
# The reference geth version (for gas arithmetic cross-check only):
#
#   go-ethereum commit: b7719e1c3de88c2e6943321fa53b80807845ba40
#   repo: github.com/ethereum/go-ethereum
#
# Source path for geth's (non-EIP-3155) wire format:
#   eth/tracers/logger/logger.go :: structLogLegacy :: toLegacyJSON
#
# TRACER NAME
# ===========
# geth does NOT expose the struct logger under any tracer-name string in its
# DefaultDirectory.  It is the implicit default when the `tracer` field is
# absent (or nil) in the debug_traceTransaction config.
#
# ethrex accepts "structLogger" (primary) and "structLog" (alias) because an
# explicit name is required for HTTP dispatch.
#
# REGENERATION PROCEDURE
# ======================
#
# Prerequisites:
#   - geth binary at commit above (build with: go build ./cmd/geth)
#   - jq for JSON pretty-printing
#   - cast (foundry) or curl for tx submission
#
# Step 1: Start geth --dev node with deterministic funded account.
#
#   geth --dev --dev.period=1 --http --http.api eth,debug,net \
#        --http.port 8545 --verbosity 1 &
#
#   GETH_PID=$!
#   FUNDED_ADDR=$(cast rpc --rpc-url http://localhost:8545 eth_accounts | jq -r '.[0]')
#   echo "Funded dev account: $FUNDED_ADDR"
#
# Step 2: Deploy each test contract.  We use `eth_sendTransaction` with `data`
# set to the init-code.  Init-code returns the runtime bytecode using a
# standard CODECOPY+RETURN pattern:
#
#   # Helper: wraps runtime bytes in a minimal deployer.
#   # Returns the deployed contract address.
#   deploy_bytecode() {
#     local runtime_hex="$1"
#     local runtime_len=$(( ${#runtime_hex} / 2 ))
#
#     # Deployer init-code pattern:
#     #   PUSH<len> <runtime>   -- push runtime length
#     #   PUSH1 0x00            -- memory dest offset
#     #   CODECOPY              -- copies runtime to mem[0]
#     #   PUSH<len> <runtime>   -- push runtime length
#     #   PUSH1 0x00
#     #   RETURN
#     # For simplicity we pre-compute this manually per fixture below.
#
#     local tx_hash=$(cast send --rpc-url http://localhost:8545 \
#       --from "$FUNDED_ADDR" --unlocked \
#       --data "0x${init_hex}" \
#       | grep transactionHash | awk '{print $2}')
#
#     cast receipt --rpc-url http://localhost:8545 "$tx_hash" \
#       | grep contractAddress | awk '{print $2}'
#   }
#
# ─── Fixture 1: eip3155_sstore_basic.json ─────────────────────────────────
#
# Bytecode: PUSH1 0x2a  PUSH1 0x01  SSTORE  STOP
#   hex: 60 2a 60 01 55 00
#
# Regenerate:
#
#   # Deploy contract with runtime bytecode 602a60015500
#   CONTRACT=$(deploy_bytecode "602a60015500")
#
#   # Call it (empty calldata, enough gas for SSTORE)
#   TX=$(cast send --rpc-url http://localhost:8545 \
#     --from "$FUNDED_ADDR" --unlocked --gas 200000 \
#     --to "$CONTRACT" | grep transactionHash | awk '{print $2}')
#
#   # Trace it (no tracer field = struct logger default in geth)
#   curl -s -X POST http://localhost:8545 \
#     -H 'Content-Type: application/json' \
#     -d "{\"jsonrpc\":\"2.0\",\"method\":\"debug_traceTransaction\",
#          \"params\":[\"$TX\",{}],\"id\":1}" \
#     | jq '.result' \
#     > test/tests/levm/fixtures/eip3155_sstore_basic.json
#
# ─── Fixture 2: eip3155_mstore_memory.json ───────────────────────────────
#
# Bytecode: PUSH1 0x20  PUSH1 0x00  MSTORE  STOP
#   hex: 60 20 60 00 52 00
#
# Regenerate:
#
#   CONTRACT=$(deploy_bytecode "602060005200")
#
#   TX=$(cast send --rpc-url http://localhost:8545 \
#     --from "$FUNDED_ADDR" --unlocked --gas 100000 \
#     --to "$CONTRACT" | grep transactionHash | awk '{print $2}')
#
#   curl -s -X POST http://localhost:8545 \
#     -H 'Content-Type: application/json' \
#     -d "{\"jsonrpc\":\"2.0\",\"method\":\"debug_traceTransaction\",
#          \"params\":[\"$TX\",{\"enableMemory\":true}],\"id\":1}" \
#     | jq '.result' \
#     > test/tests/levm/fixtures/eip3155_mstore_memory.json
#
# ─── Fixture 3: eip3155_identity_return_data.json ────────────────────────
#
# Calls identity precompile (0x04) via STATICCALL with 1 byte input.
# Contract returns input unchanged, demonstrating returnData on the next step.
#
# Bytecode (18 bytes):
#   PUSH1 0x01  PUSH1 0x00  MSTORE8    -- write 0x01 to mem[0]
#   PUSH1 0x01  PUSH1 0x00             -- retLen=1, retOffset=0
#   PUSH1 0x01  PUSH1 0x00             -- argsLen=1, argsOffset=0
#   PUSH1 0x04                         -- addr=identity
#   GAS         STATICCALL
#   STOP
#   hex: 6001600053600160006001600060045afa00
#
# Regenerate:
#
#   CONTRACT=$(deploy_bytecode "6001600053600160006001600060045afa00")
#
#   TX=$(cast send --rpc-url http://localhost:8545 \
#     --from "$FUNDED_ADDR" --unlocked --gas 100000 \
#     --to "$CONTRACT" | grep transactionHash | awk '{print $2}')
#
#   curl -s -X POST http://localhost:8545 \
#     -H 'Content-Type: application/json' \
#     -d "{\"jsonrpc\":\"2.0\",\"method\":\"debug_traceTransaction\",
#          \"params\":[\"$TX\",{\"enableReturnData\":true}],\"id\":1}" \
#     | jq '.result' \
#     > test/tests/levm/fixtures/eip3155_identity_return_data.json
#
# ─── Cleanup ──────────────────────────────────────────────────────────────
#
#   kill $GETH_PID
#
# IMPORTANT: after regenerating, update the gas values in the fixture files.
# The exact gas figures depend on the base fee and the gas_limit parameter
# sent in the transaction; keep them consistent with how the test helper in
# test/tests/levm/struct_log_tracer_tests.rs sets up the EIP-1559 tx
# (gas_limit=100_000, base_fee=1, max_fee=10).

set -euo pipefail

echo "This script documents the fixture-regeneration procedure only."
echo "See the comments above for the full step-by-step instructions."
echo ""
echo "Pinned geth commit: b7719e1c3de88c2e6943321fa53b80807845ba40"
echo "Fixtures location: test/tests/levm/fixtures/eip3155_*.json"
