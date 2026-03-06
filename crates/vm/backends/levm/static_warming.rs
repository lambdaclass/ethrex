//! Static warming module - replaces speculative transaction re-execution with
//! static analysis to pre-warm state before block execution.
//!
//! This module extracts account and storage access patterns directly from
//! transaction data and bytecode, avoiding the need to run the EVM.

use ethrex_common::{Address, H256};
use ethrex_common::types::{Block, Transaction, TxKind};
use ethrex_levm::db::Database;
use std::collections::HashMap;
use std::sync::Arc;

/// Opcode values for bytecode analysis
const OPCODE_PUSH1: u8 = 0x60;
const OPCODE_PUSH2: u8 = 0x61;
const OPCODE_SLOAD: u8 = 0x54;

/// Transaction with sender recovered
type TxWithSender<'a> = (&'a Transaction, Address);

/// Extract all call target addresses from transactions.
/// Filters out CREATE transactions (where tx.to() is None).
pub fn extract_call_targets<'a>(txs: &[TxWithSender<'a>]) -> Vec<Address> {
    let mut targets = Vec::new();
    for (tx, _sender) in txs {
        if let TxKind::Call(address) = tx.to() {
            targets.push(address);
        }
        // CREATE transactions have TxKind::Create, we handle those separately
    }
    targets
}

/// Predict addresses that will be created by CREATE transactions.
///
/// For CREATE: address = keccak256(rlp([sender, nonce]))[12:]
///
/// Note: CREATE2 is not handled here as it requires access to the initcode
/// which is not available until execution time.
pub fn predict_create_addresses(txs: &[TxWithSender]) -> Vec<Address> {
    let mut addresses = Vec::new();

    // Estimate nonce for each sender based on transaction position
    // This is a heuristic - real nonces may differ if some txs fail
    let mut sender_tx_index: HashMap<Address, u64> = HashMap::new();

    for (tx, sender) in txs {
        if tx.to() == TxKind::Create {
            // CREATE: address = keccak256(rlp([sender, nonce]))[12:]
            let estimated_nonce = *sender_tx_index.get(sender).unwrap_or(&0);
            let create_address = ethrex_common::evm::calculate_create_address(*sender, estimated_nonce);
            addresses.push(create_address);
        }
        // Increment estimated nonce for this sender (both Create and Call increment nonce)
        *sender_tx_index.entry(*sender).or_insert(0) += 1;
    }

    addresses
}

/// Extract static storage keys from bytecode by scanning for PUSH + SLOAD patterns.
/// This catches common patterns like:
/// - PUSH1 <slot> SLOAD
/// - PUSH2 <slot> SLOAD
pub fn extract_static_storage_keys(code: &[u8]) -> Vec<H256> {
    let mut keys = Vec::new();

    // Need at least 3 bytes for PUSH1 + SLOAD (PUSH + slot + SLOAD)
    if code.len() < 3 {
        return keys;
    }

    for i in 0..code.len() - 2 {
        // PUSH1 (0x60) + 1-byte slot + SLOAD (0x54)
        if code[i] == OPCODE_PUSH1 && code[i + 2] == OPCODE_SLOAD {
            let slot = code[i + 1] as u64;
            keys.push(H256::from_low_u64_be(slot));
        }
        // PUSH2 (0x61) + 2-byte slot + SLOAD (0x54)
        else if code[i] == OPCODE_PUSH2 && i + 3 < code.len() && code[i + 3] == OPCODE_SLOAD {
            let hi = code[i + 1] as u64;
            let lo = code[i + 2] as u64;
            let slot = hi * 256 + lo;
            keys.push(H256::from_low_u64_be(slot));
        }
    }

    keys
}

/// Pre-warm state using static analysis instead of speculative re-execution.
/// This is faster than warm_block() but may miss some dynamic storage accesses.
pub fn warm_block_static(
    block: &Block,
    store: Arc<dyn Database>,
) -> Result<(), String> {
    // Get transactions with senders
    let txs_with_sender = block.body.get_transactions_with_sender()
        .map_err(|e| format!("Failed to recover tx senders: {}", e))?;

    // 1. Extract call targets
    let call_targets = extract_call_targets(&txs_with_sender);

    // 2. Predict CREATE addresses
    let create_addresses = predict_create_addresses(&txs_with_sender);

    // 3. Also add all senders (they'll definitely be accessed for nonce/balance)
    let mut all_addresses: Vec<Address> = call_targets;
    all_addresses.extend(create_addresses);
    for (_tx, sender) in &txs_with_sender {
        if !all_addresses.contains(sender) {
            all_addresses.push(*sender);
        }
    }

    // Deduplicate
    all_addresses.sort();
    all_addresses.dedup();

    // 4. Batch prefetch accounts
    if !all_addresses.is_empty() {
        store.prefetch_accounts(&all_addresses)
            .map_err(|e| format!("Failed to prefetch accounts: {}", e))?;
    }

    // 6. For each contract with code, analyze bytecode for storage keys
    let mut storage_keys: Vec<(Address, H256)> = Vec::new();

    for &addr in &all_addresses {
        // Get account state to check if code exists
        if let Ok(account_state) = store.get_account_state(addr) {
            // Skip if no code
            if account_state.code_hash == *ethrex_common::constants::EMPTY_KECCACK_HASH {
                continue;
            }

            // Get code
            if let Ok(code) = store.get_account_code(account_state.code_hash) {
                // Extract static storage keys
                let keys = extract_static_storage_keys(&code.bytecode);
                for key in keys {
                    storage_keys.push((addr, key));
                }
            }
        }
    }

    // 7. Batch prefetch storage slots
    if !storage_keys.is_empty() {
        store.prefetch_storage(&storage_keys)
            .map_err(|e| format!("Failed to prefetch storage: {}", e))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{Transaction as Tx, TxKind};

    #[test]
    fn test_extract_storage_keys_push1_sload() {
        // PUSH1 0x42 SLOAD
        let code = vec![0x60, 0x42, 0x54];
        let keys = extract_static_storage_keys(&code);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], H256::from_low_u64_be(0x42));
    }

    #[test]
    fn test_extract_storage_keys_push2_sload() {
        // PUSH2 0x1234 SLOAD
        let code = vec![0x61, 0x12, 0x34, 0x54];
        let keys = extract_static_storage_keys(&code);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], H256::from_low_u64_be(0x1234));
    }

    #[test]
    fn test_extract_storage_keys_no_sload() {
        // PUSH1 0x42 PUSH2 0x1234 (no SLOAD)
        let code = vec![0x60, 0x42, 0x61, 0x12, 0x34];
        let keys = extract_static_storage_keys(&code);
        assert!(keys.is_empty());
    }

    #[test]
    fn test_extract_storage_keys_empty() {
        let keys = extract_static_storage_keys(&[]);
        assert!(keys.is_empty());

        let keys = extract_static_storage_keys(&[0x54]); // Just SLOAD
        assert!(keys.is_empty());
    }

    #[test]
    fn test_predict_create_addresses_returns_empty_for_mock_data() {
        // This test verifies the function compiles and returns expected type
        // Full integration testing requires actual block data
        let sender = Address::from_low_u64_be(0x1234);
        let txs_with_sender: Vec<(&Tx, Address)> = vec![];
        
        let addresses = predict_create_addresses(&txs_with_sender);
        assert!(addresses.is_empty());
    }
}
