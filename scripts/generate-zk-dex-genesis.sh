#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# Generate L2 Genesis JSON with ZkDex contracts pre-deployed
#
# This script:
# 1. Compiles ZkDex contracts using Forge (in the zk-dex project)
# 2. Extracts deployedBytecode for 6 verifiers + ZkDex
# 3. Verifies ZkDex storage layout matches expected slot assignments
# 4. Merges contracts into the base L2 genesis JSON
# 5. Outputs fixtures/genesis/l2-zk-dex.json
#
# Prerequisites:
#   - forge (foundry) installed
#   - jq installed
#   - zk-dex project with compiled Groth16 verifier contracts
#     (run circuits-circom compile -> setup -> generate_verifiers first)
#
# Usage: ./scripts/generate-zk-dex-genesis.sh [--zk-dex-dir PATH]
# =============================================================================

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZK_DEX_DIR="${ZK_DEX_DIR:-$(cd "$REPO_ROOT/../zk-dex" && pwd)}"
BASE_GENESIS="$REPO_ROOT/fixtures/genesis/l2.json"
OUTPUT_GENESIS="$REPO_ROOT/fixtures/genesis/l2-zk-dex.json"

# Contract addresses (matching guest program hardcoded addresses)
ZKDEX_ADDR="0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE"
MINT_BURN_VERIFIER_ADDR="0xDE00000000000000000000000000000000000001"
TRANSFER_VERIFIER_ADDR="0xDE00000000000000000000000000000000000002"
CONVERT_VERIFIER_ADDR="0xDE00000000000000000000000000000000000003"
MAKE_ORDER_VERIFIER_ADDR="0xDE00000000000000000000000000000000000004"
TAKE_ORDER_VERIFIER_ADDR="0xDE00000000000000000000000000000000000005"
SETTLE_ORDER_VERIFIER_ADDR="0xDE00000000000000000000000000000000000006"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --zk-dex-dir)
            ZK_DEX_DIR="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1"
            exit 1
            ;;
    esac
done

# =============================================================================
# Validation
# =============================================================================

log_info() { echo "[INFO] $*"; }
log_error() { echo "[ERROR] $*" >&2; }

if ! command -v forge &> /dev/null; then
    log_error "forge (foundry) is not installed"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    log_error "jq is not installed"
    exit 1
fi

if [[ ! -f "$BASE_GENESIS" ]]; then
    log_error "Base genesis not found: $BASE_GENESIS"
    exit 1
fi

if [[ ! -d "$ZK_DEX_DIR" ]]; then
    log_error "ZK-DEX project not found: $ZK_DEX_DIR"
    exit 1
fi

if [[ ! -f "$ZK_DEX_DIR/foundry.toml" ]]; then
    log_error "foundry.toml not found in $ZK_DEX_DIR. Please create it first."
    exit 1
fi

# =============================================================================
# Step 1: Build contracts with Forge
# =============================================================================

log_info "Building ZkDex contracts with Forge..."
(cd "$ZK_DEX_DIR" && forge build --force)

# =============================================================================
# Step 2: Extract deployedBytecode
# =============================================================================

log_info "Extracting deployed bytecodes..."

get_deployed_bytecode() {
    local contract_name="$1"
    local bytecode
    bytecode=$(cd "$ZK_DEX_DIR" && forge inspect "$contract_name" deployedBytecode)
    if [[ -z "$bytecode" || "$bytecode" == "0x" ]]; then
        log_error "Failed to get deployedBytecode for $contract_name"
        exit 1
    fi
    echo "$bytecode"
}

ZKDEX_CODE=$(get_deployed_bytecode "ZkDex")
MINT_BURN_CODE=$(get_deployed_bytecode "MintBurnNoteVerifier")
TRANSFER_CODE=$(get_deployed_bytecode "TransferNoteVerifier")
CONVERT_CODE=$(get_deployed_bytecode "ConvertNoteVerifier")
MAKE_ORDER_CODE=$(get_deployed_bytecode "MakeOrderVerifier")
TAKE_ORDER_CODE=$(get_deployed_bytecode "TakeOrderVerifier")
SETTLE_ORDER_CODE=$(get_deployed_bytecode "SettleOrderVerifier")

log_info "All bytecodes extracted successfully"

# =============================================================================
# Step 3: Verify ZkDex storage layout
# =============================================================================

log_info "Verifying ZkDex storage layout..."

STORAGE_LAYOUT=$(cd "$ZK_DEX_DIR" && forge inspect ZkDex storage-layout --json)

verify_slot() {
    local var_name="$1"
    local expected_slot="$2"
    local actual_slot
    actual_slot=$(echo "$STORAGE_LAYOUT" | jq -r ".storage[] | select(.label == \"$var_name\") | .slot")

    if [[ "$actual_slot" != "$expected_slot" ]]; then
        log_error "Storage slot mismatch for $var_name: expected=$expected_slot, actual=$actual_slot"
        log_error "Full storage layout:"
        echo "$STORAGE_LAYOUT" | jq '.storage[] | {label: .label, slot: .slot}' >&2
        exit 1
    fi
    log_info "  $var_name -> slot $actual_slot (OK)"
}

verify_slot "development" "0"
verify_slot "requestVerifier" "1"
verify_slot "mintNoteVerifier" "6"
verify_slot "spendNoteVerifier" "7"
verify_slot "liquidateNoteVerifier" "8"
verify_slot "convertNoteVerifier" "9"
verify_slot "makeOrderVerifier" "10"
verify_slot "takeOrderVerifier" "11"
verify_slot "settleOrderVerifier" "12"
verify_slot "orders" "13"

log_info "Storage layout verification passed!"

# =============================================================================
# Step 4: Generate genesis JSON
# =============================================================================

log_info "Generating l2-zk-dex.json..."

# Helper: left-pad address to 32-byte hex for storage value
addr_to_storage_value() {
    local addr="$1"
    # Remove 0x prefix, lowercase, left-pad to 64 chars
    local clean
    clean=$(echo "${addr#0x}" | tr '[:upper:]' '[:lower:]')
    printf "0x%064s" "$clean" | tr ' ' '0'
}

# Storage values for ZkDex (verified by forge inspect)
# Slot 0: development(false) + dai(0x0) packed = 0x0
SLOT_0="0x0000000000000000000000000000000000000000000000000000000000000000"
# Slot 1: requestVerifier = MintBurnNoteVerifier
SLOT_1=$(addr_to_storage_value "$MINT_BURN_VERIFIER_ADDR")
# Slot 6: mintNoteVerifier = MintBurnNoteVerifier
SLOT_6=$(addr_to_storage_value "$MINT_BURN_VERIFIER_ADDR")
# Slot 7: spendNoteVerifier = TransferNoteVerifier
SLOT_7=$(addr_to_storage_value "$TRANSFER_VERIFIER_ADDR")
# Slot 8: liquidateNoteVerifier = MintBurnNoteVerifier
SLOT_8=$(addr_to_storage_value "$MINT_BURN_VERIFIER_ADDR")
# Slot 9: convertNoteVerifier = ConvertNoteVerifier
SLOT_9=$(addr_to_storage_value "$CONVERT_VERIFIER_ADDR")
# Slot 10: makeOrderVerifier = MakeOrderVerifier
SLOT_10=$(addr_to_storage_value "$MAKE_ORDER_VERIFIER_ADDR")
# Slot 11: takeOrderVerifier = TakeOrderVerifier
SLOT_11=$(addr_to_storage_value "$TAKE_ORDER_VERIFIER_ADDR")
# Slot 12: settleOrderVerifier = SettleOrderVerifier
SLOT_12=$(addr_to_storage_value "$SETTLE_ORDER_VERIFIER_ADDR")
# Slot 13: orders.length = 0
SLOT_13="0x0000000000000000000000000000000000000000000000000000000000000000"

# Build the jq filter to add all 7 contracts to alloc
# Verifier contracts: just code, no storage needed
# ZkDex: code + storage
jq \
    --arg zkdex_addr "$ZKDEX_ADDR" \
    --arg zkdex_code "$ZKDEX_CODE" \
    --arg mint_burn_addr "$MINT_BURN_VERIFIER_ADDR" \
    --arg mint_burn_code "$MINT_BURN_CODE" \
    --arg transfer_addr "$TRANSFER_VERIFIER_ADDR" \
    --arg transfer_code "$TRANSFER_CODE" \
    --arg convert_addr "$CONVERT_VERIFIER_ADDR" \
    --arg convert_code "$CONVERT_CODE" \
    --arg make_order_addr "$MAKE_ORDER_VERIFIER_ADDR" \
    --arg make_order_code "$MAKE_ORDER_CODE" \
    --arg take_order_addr "$TAKE_ORDER_VERIFIER_ADDR" \
    --arg take_order_code "$TAKE_ORDER_CODE" \
    --arg settle_order_addr "$SETTLE_ORDER_VERIFIER_ADDR" \
    --arg settle_order_code "$SETTLE_ORDER_CODE" \
    --arg slot_0 "$SLOT_0" \
    --arg slot_1 "$SLOT_1" \
    --arg slot_6 "$SLOT_6" \
    --arg slot_7 "$SLOT_7" \
    --arg slot_8 "$SLOT_8" \
    --arg slot_9 "$SLOT_9" \
    --arg slot_10 "$SLOT_10" \
    --arg slot_11 "$SLOT_11" \
    --arg slot_12 "$SLOT_12" \
    --arg slot_13 "$SLOT_13" \
    '
    # Add 6 verifier contracts (code only, no storage)
    .alloc[($mint_burn_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $mint_burn_code,
        "nonce": "0x1",
        "storage": {}
    }
    | .alloc[($transfer_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $transfer_code,
        "nonce": "0x1",
        "storage": {}
    }
    | .alloc[($convert_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $convert_code,
        "nonce": "0x1",
        "storage": {}
    }
    | .alloc[($make_order_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $make_order_code,
        "nonce": "0x1",
        "storage": {}
    }
    | .alloc[($take_order_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $take_order_code,
        "nonce": "0x1",
        "storage": {}
    }
    | .alloc[($settle_order_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $settle_order_code,
        "nonce": "0x1",
        "storage": {}
    }
    # Add ZkDex main contract with storage layout
    | .alloc[($zkdex_addr | ascii_downcase)] = {
        "balance": "0x0",
        "code": $zkdex_code,
        "nonce": "0x1",
        "storage": {
            "0x0000000000000000000000000000000000000000000000000000000000000000": $slot_0,
            "0x0000000000000000000000000000000000000000000000000000000000000001": $slot_1,
            "0x0000000000000000000000000000000000000000000000000000000000000006": $slot_6,
            "0x0000000000000000000000000000000000000000000000000000000000000007": $slot_7,
            "0x0000000000000000000000000000000000000000000000000000000000000008": $slot_8,
            "0x0000000000000000000000000000000000000000000000000000000000000009": $slot_9,
            "0x000000000000000000000000000000000000000000000000000000000000000a": $slot_10,
            "0x000000000000000000000000000000000000000000000000000000000000000b": $slot_11,
            "0x000000000000000000000000000000000000000000000000000000000000000c": $slot_12,
            "0x000000000000000000000000000000000000000000000000000000000000000d": $slot_13
        }
    }
    ' "$BASE_GENESIS" > "$OUTPUT_GENESIS"

# =============================================================================
# Step 5: Verify output
# =============================================================================

if [[ ! -f "$OUTPUT_GENESIS" ]]; then
    log_error "Failed to generate $OUTPUT_GENESIS"
    exit 1
fi

# Verify it's valid JSON
if ! jq empty "$OUTPUT_GENESIS" 2>/dev/null; then
    log_error "Generated JSON is invalid"
    rm -f "$OUTPUT_GENESIS"
    exit 1
fi

# Count alloc entries
BASE_COUNT=$(jq '.alloc | length' "$BASE_GENESIS")
OUTPUT_COUNT=$(jq '.alloc | length' "$OUTPUT_GENESIS")
ADDED=$((OUTPUT_COUNT - BASE_COUNT))

# Verify ZkDex code is present
ZKDEX_CODE_CHECK=$(jq -r ".alloc[\"$(echo "$ZKDEX_ADDR" | tr '[:upper:]' '[:lower:]')\"].code // empty" "$OUTPUT_GENESIS")
if [[ -z "$ZKDEX_CODE_CHECK" ]]; then
    log_error "ZkDex code not found in genesis"
    exit 1
fi

log_info ""
log_info "========================================="
log_info "  ZK-DEX Genesis Generated Successfully"
log_info "========================================="
log_info "  Output:   $OUTPUT_GENESIS"
log_info "  Base alloc entries: $BASE_COUNT"
log_info "  New alloc entries:  $OUTPUT_COUNT (+$ADDED)"
log_info ""
log_info "  Contracts:"
log_info "    MintBurnNoteVerifier:  $MINT_BURN_VERIFIER_ADDR"
log_info "    TransferNoteVerifier:  $TRANSFER_VERIFIER_ADDR"
log_info "    ConvertNoteVerifier:   $CONVERT_VERIFIER_ADDR"
log_info "    MakeOrderVerifier:     $MAKE_ORDER_VERIFIER_ADDR"
log_info "    TakeOrderVerifier:     $TAKE_ORDER_VERIFIER_ADDR"
log_info "    SettleOrderVerifier:   $SETTLE_ORDER_VERIFIER_ADDR"
log_info "    ZkDex:                 $ZKDEX_ADDR"
log_info "========================================="
