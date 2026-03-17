//! Convert `ProgramInput` → `AppProgramInput`.
//!
//! The prover pipeline produces `ProgramInput` (containing a full
//! `ExecutionWitness` with embedded trie nodes). App-specific guest programs
//! (e.g., zk-dex) need `AppProgramInput` (containing only Merkle proofs for
//! the accounts/slots they touch).
//!
//! This module bridges the gap by:
//! 1. Rebuilding state & storage tries from the `ExecutionWitness`
//! 2. Extracting Merkle proofs for the requested accounts and storage slots
//! 3. Assembling the result into an `AppProgramInput`

use std::collections::BTreeSet;

use ethrex_common::types::AccountState;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::Trie;

use super::app_types::{AccountProof, AppProgramInput, StorageProof};
use crate::l2::ProgramInput;

/// Errors during `ProgramInput` → `AppProgramInput` conversion.
#[derive(Debug, thiserror::Error)]
pub enum InputConversionError {
    #[error("Trie reconstruction failed: {0}")]
    TrieReconstruction(String),
    #[error("Account not found in witness: {0:?}")]
    AccountNotFound(Address),
    #[error("Storage trie not found for: {0:?}")]
    StorageTrieNotFound(Address),
    #[error("RLP decode error: {0}")]
    RlpDecode(String),
    #[error("Trie error: {0}")]
    TrieError(String),
}

/// Convert a `ProgramInput` into an `AppProgramInput` by extracting Merkle
/// proofs for the specified accounts and storage slots.
///
/// # Arguments
///
/// * `input` — The full prover input containing an `ExecutionWitness`.
/// * `needed_accounts` — Addresses for which account proofs are required.
/// * `needed_storage` — `(address, slot)` pairs for which storage proofs are required.
pub fn convert_to_app_input(
    input: ProgramInput,
    needed_accounts: &[Address],
    needed_storage: &[(Address, H256)],
) -> Result<AppProgramInput, InputConversionError> {
    let witness = &input.execution_witness;

    // 1. Rebuild state trie from the ExecutionWitness.
    let state_trie = rebuild_state_trie(witness)?;
    let prev_state_root = state_trie.hash_no_commit();

    // 2. Rebuild storage tries.
    let storage_tries = rebuild_storage_tries(witness)?;

    // 3. Collect all addresses that need account proofs (union of
    //    needed_accounts and addresses from needed_storage).
    let all_account_addrs: BTreeSet<Address> = needed_accounts
        .iter()
        .copied()
        .chain(needed_storage.iter().map(|(addr, _)| *addr))
        .collect();

    // 4. Extract account proofs.
    let mut account_proofs = Vec::with_capacity(all_account_addrs.len());
    for address in &all_account_addrs {
        let hashed_addr = keccak_hash(address.as_bytes()).to_vec();

        let proof = state_trie
            .get_proof(&hashed_addr)
            .map_err(|e| InputConversionError::TrieError(e.to_string()))?;

        // Decode the account state from the trie.
        let account_state = match state_trie
            .get(&hashed_addr)
            .map_err(|e| InputConversionError::TrieError(e.to_string()))?
        {
            Some(rlp) => AccountState::decode(&rlp)
                .map_err(|e| InputConversionError::RlpDecode(e.to_string()))?,
            None => {
                // Account not in trie — use defaults (nonce=0, balance=0, etc.)
                AccountState::default()
            }
        };

        account_proofs.push(AccountProof {
            address: *address,
            nonce: account_state.nonce,
            balance: account_state.balance,
            storage_root: account_state.storage_root,
            code_hash: account_state.code_hash,
            proof,
        });
    }

    // 5. Extract storage proofs.
    let mut storage_proofs = Vec::with_capacity(needed_storage.len());
    for (address, slot) in needed_storage {
        let hashed_slot = keccak_hash(slot.as_bytes()).to_vec();

        // Account proof is also needed for each storage proof.
        let hashed_addr = keccak_hash(address.as_bytes()).to_vec();
        let account_proof = state_trie
            .get_proof(&hashed_addr)
            .map_err(|e| InputConversionError::TrieError(e.to_string()))?;

        // Get the storage trie for this account.
        let storage_trie = storage_tries
            .get(address)
            .ok_or(InputConversionError::StorageTrieNotFound(*address))?;

        let storage_proof = storage_trie
            .get_proof(&hashed_slot)
            .map_err(|e| InputConversionError::TrieError(e.to_string()))?;

        // Read the current value.
        let value = match storage_trie
            .get(&hashed_slot)
            .map_err(|e| InputConversionError::TrieError(e.to_string()))?
        {
            Some(rlp) => {
                U256::decode(&rlp).map_err(|e| InputConversionError::RlpDecode(e.to_string()))?
            }
            None => U256::zero(),
        };

        storage_proofs.push(StorageProof {
            address: *address,
            slot: *slot,
            value,
            account_proof,
            storage_proof,
        });
    }

    // 6. Assemble AppProgramInput.
    Ok(AppProgramInput {
        blocks: input.blocks,
        prev_state_root,
        storage_proofs,
        account_proofs,
        elasticity_multiplier: input.elasticity_multiplier,
        fee_configs: input.fee_configs,
        blob_commitment: input.blob_commitment,
        blob_proof: input.blob_proof,
        chain_id: input.execution_witness.chain_config.chain_id,
    })
}

/// Rebuild the state trie from an `ExecutionWitness`.
fn rebuild_state_trie(witness: &ExecutionWitness) -> Result<Trie, InputConversionError> {
    let trie = if let Some(ref state_trie_root) = witness.state_trie_root {
        Trie::new_temp_with_root(state_trie_root.clone().into())
    } else {
        Trie::new_temp()
    };
    trie.hash_no_commit();
    Ok(trie)
}

/// Rebuild per-account storage tries from an `ExecutionWitness`.
fn rebuild_storage_tries(
    witness: &ExecutionWitness,
) -> Result<std::collections::BTreeMap<Address, Trie>, InputConversionError> {
    let mut storage_tries = std::collections::BTreeMap::new();
    for (address, storage_trie_root) in &witness.storage_trie_roots {
        let storage_trie = Trie::new_temp_with_root(storage_trie_root.clone().into());
        storage_trie.hash_no_commit();
        storage_tries.insert(*address, storage_trie);
    }
    Ok(storage_tries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ethrex_common::types::block_execution_witness::ExecutionWitness;
    use ethrex_common::types::{AccountState, ChainConfig};
    use ethrex_common::{H160, H256, U256};
    use ethrex_rlp::encode::RLPEncode as _;
    use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie};

    fn test_address(byte: u8) -> Address {
        H160([byte; 20])
    }

    fn test_account(nonce: u64, balance: u64) -> AccountState {
        AccountState {
            nonce,
            balance: U256::from(balance),
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: H256::zero(),
        }
    }

    /// Extract the root Node from a Trie for building test ExecutionWitness.
    fn extract_root_node(trie: &Trie) -> Node {
        trie.root_node()
            .expect("root_node")
            .map(Arc::unwrap_or_clone)
            .expect("trie should have a root node")
    }

    /// Build a simple ExecutionWitness with accounts in the state trie.
    fn make_witness_with_accounts(
        accounts: Vec<(Address, AccountState)>,
    ) -> (ExecutionWitness, H256) {
        let mut state_trie = Trie::empty_in_memory();
        for (addr, state) in &accounts {
            let path = keccak_hash(addr.as_bytes()).to_vec();
            state_trie
                .insert(path, state.encode_to_vec())
                .expect("insert");
        }
        let root = state_trie.hash_no_commit();

        let root_node = extract_root_node(&state_trie);

        let witness = ExecutionWitness {
            state_trie_root: Some(root_node),
            chain_config: ChainConfig {
                chain_id: 42,
                ..Default::default()
            },
            ..Default::default()
        };

        (witness, root)
    }

    /// Build a ProgramInput from a witness + optional blocks.
    fn make_program_input(witness: ExecutionWitness) -> ProgramInput {
        ProgramInput {
            blocks: vec![],
            execution_witness: witness,
            elasticity_multiplier: 2,
            fee_configs: vec![],
            blob_commitment: [0u8; 48],
            blob_proof: [0u8; 48],
            native_token_scale_factor: None,
        }
    }

    #[test]
    fn test_convert_single_account() {
        let addr = test_address(0x01);
        let account = test_account(5, 1000);
        let (witness, _root) = make_witness_with_accounts(vec![(addr, account)]);

        let input = make_program_input(witness);
        let result = convert_to_app_input(input, &[addr], &[]);
        assert!(
            result.is_ok(),
            "conversion should succeed: {:?}",
            result.err()
        );

        let app_input = result.unwrap();
        assert_eq!(app_input.account_proofs.len(), 1);

        let ap = &app_input.account_proofs[0];
        assert_eq!(ap.address, addr);
        assert_eq!(ap.nonce, 5);
        assert_eq!(ap.balance, U256::from(1000));
        assert!(!ap.proof.is_empty(), "proof should not be empty");
    }

    #[test]
    fn test_prev_state_root_matches() {
        let addr = test_address(0x01);
        let account = test_account(0, 500);
        let (witness, expected_root) = make_witness_with_accounts(vec![(addr, account)]);

        let input = make_program_input(witness);
        let app_input = convert_to_app_input(input, &[addr], &[]).unwrap();

        assert_eq!(
            app_input.prev_state_root, expected_root,
            "prev_state_root should match the trie root"
        );
    }

    #[test]
    fn test_fields_are_copied() {
        let addr = test_address(0x01);
        let account = test_account(0, 0);
        let (witness, _) = make_witness_with_accounts(vec![(addr, account)]);

        let mut input = make_program_input(witness);
        input.elasticity_multiplier = 7;
        input.blob_commitment = [0xAA; 48];
        input.blob_proof = [0xBB; 48];

        let app_input = convert_to_app_input(input, &[addr], &[]).unwrap();

        assert_eq!(app_input.elasticity_multiplier, 7);
        assert_eq!(app_input.blob_commitment, [0xAA; 48]);
        assert_eq!(app_input.blob_proof, [0xBB; 48]);
        assert_eq!(app_input.chain_id, 42);
        assert!(app_input.blocks.is_empty());
        assert!(app_input.fee_configs.is_empty());
    }

    #[test]
    fn test_unknown_account_uses_defaults() {
        // If an account is not in the trie, we get default values.
        let addr_in_trie = test_address(0x01);
        let account = test_account(1, 100);
        let (witness, _) = make_witness_with_accounts(vec![(addr_in_trie, account)]);

        let unknown_addr = test_address(0xFF);
        let input = make_program_input(witness);
        let result = convert_to_app_input(input, &[unknown_addr], &[]);

        // Should succeed — unknown accounts get default AccountState.
        assert!(result.is_ok());
        let app_input = result.unwrap();
        assert_eq!(app_input.account_proofs.len(), 1);
        assert_eq!(app_input.account_proofs[0].nonce, 0);
        assert_eq!(app_input.account_proofs[0].balance, U256::zero());
    }

    #[test]
    fn test_convert_with_storage() {
        let contract = test_address(0xDE);
        let slot = H256::from_low_u64_be(42);
        let value = U256::from(9999);

        // Build a storage trie for the contract.
        let mut storage_trie = Trie::empty_in_memory();
        let hashed_slot = keccak_hash(slot.as_bytes()).to_vec();
        storage_trie
            .insert(hashed_slot, value.encode_to_vec())
            .expect("insert");
        let storage_root = storage_trie.hash_no_commit();
        let storage_root_node = extract_root_node(&storage_trie);

        // Build the account with the correct storage root.
        let account = AccountState {
            nonce: 0,
            balance: U256::zero(),
            storage_root,
            code_hash: H256::zero(),
        };

        // Build the state trie with the contract account.
        let mut state_trie = Trie::empty_in_memory();
        let hashed_addr = keccak_hash(contract.as_bytes()).to_vec();
        state_trie
            .insert(hashed_addr, account.encode_to_vec())
            .expect("insert");
        let _state_root = state_trie.hash_no_commit();
        let state_root_node = extract_root_node(&state_trie);

        let mut storage_trie_roots = std::collections::BTreeMap::new();
        storage_trie_roots.insert(contract, storage_root_node);

        let witness = ExecutionWitness {
            state_trie_root: Some(state_root_node),
            storage_trie_roots,
            chain_config: ChainConfig {
                chain_id: 1,
                ..Default::default()
            },
            ..Default::default()
        };

        let input = make_program_input(witness);
        let result = convert_to_app_input(input, &[contract], &[(contract, slot)]);
        assert!(
            result.is_ok(),
            "conversion with storage should succeed: {:?}",
            result.err()
        );

        let app_input = result.unwrap();

        // Account proof should exist.
        assert_eq!(app_input.account_proofs.len(), 1);
        assert_eq!(app_input.account_proofs[0].address, contract);
        assert_eq!(app_input.account_proofs[0].storage_root, storage_root);

        // Storage proof should exist.
        assert_eq!(app_input.storage_proofs.len(), 1);
        let sp = &app_input.storage_proofs[0];
        assert_eq!(sp.address, contract);
        assert_eq!(sp.slot, slot);
        assert_eq!(sp.value, value);
        assert!(!sp.account_proof.is_empty());
        assert!(!sp.storage_proof.is_empty());
    }

    #[test]
    fn test_storage_trie_not_found_returns_error() {
        let addr = test_address(0x01);
        let account = test_account(0, 0);
        let (witness, _) = make_witness_with_accounts(vec![(addr, account)]);

        let slot = H256::from_low_u64_be(1);
        let input = make_program_input(witness);
        // Request storage for an address with no storage trie in the witness.
        let result = convert_to_app_input(input, &[], &[(addr, slot)]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, InputConversionError::StorageTrieNotFound(_)),
            "expected StorageTrieNotFound, got: {err}"
        );
    }
}
