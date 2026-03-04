//! Incremental Merkle Patricia Trie (MPT) operations for app-specific circuits.
//!
//! Instead of maintaining a full state trie and re-executing all EVM operations,
//! the app-specific circuit receives Merkle proofs for specific accounts and
//! storage slots, verifies them against the previous state root, applies updates,
//! and recomputes the new state root.
//!
//! This is dramatically cheaper than full EVM execution because:
//! - Only the affected paths are hashed (not the entire trie)
//! - No EVM interpreter overhead
//! - Number of keccak256 calls ≈ (# changed slots) × (trie depth ≈ 15)

use ethrex_common::types::AccountState;
use ethrex_common::{Address, H256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{EMPTY_TRIE_HASH, InMemoryTrieDB, Nibbles, Node, Trie, TrieDB};

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::app_state::AppState;

/// Errors during incremental MPT operations.
#[derive(Debug, thiserror::Error)]
pub enum IncrementalMptError {
    #[error("Proof verification failed for account {0:?}: computed root {1:?} != expected {2:?}")]
    AccountProofMismatch(Address, H256, H256),
    #[error("Proof verification failed for storage slot {address:?}/{slot:?}")]
    StorageProofMismatch { address: Address, slot: H256 },
    #[error("Trie error: {0}")]
    Trie(String),
    #[error("RLP decode error: {0}")]
    RlpDecode(String),
}

/// Verify all proofs in the AppState against the previous state root.
///
/// This must be called before executing any app operations to ensure
/// the state values are authentic.
pub fn verify_state_proofs(state: &AppState) -> Result<(), IncrementalMptError> {
    let prev_root = state.prev_state_root();

    // Nothing to verify when there are no proofs (e.g., empty batch).
    if state.account_proofs().is_empty() {
        return Ok(());
    }

    // Build a partial state trie from account proofs and verify root.
    let state_trie = build_trie_from_proofs(
        prev_root,
        state
            .account_proofs()
            .iter()
            .map(|ap| {
                let path = keccak_hash(ap.address.as_bytes()).to_vec();
                (path, ap.proof.clone())
            })
            .collect(),
    )?;

    let computed_root = state_trie.hash_no_commit();
    if computed_root != prev_root {
        return Err(IncrementalMptError::AccountProofMismatch(
            Address::zero(),
            computed_root,
            prev_root,
        ));
    }

    // Verify each account's value matches its proof.
    for ap in state.account_proofs() {
        let path = keccak_hash(ap.address.as_bytes()).to_vec();
        let stored_value = state_trie
            .get(&path)
            .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;

        if let Some(stored_rlp) = stored_value {
            let stored_account = AccountState::decode(&stored_rlp)
                .map_err(|e| IncrementalMptError::RlpDecode(e.to_string()))?;
            let expected = AccountState {
                nonce: ap.nonce,
                balance: ap.balance,
                storage_root: ap.storage_root,
                code_hash: ap.code_hash,
            };
            if stored_account != expected {
                return Err(IncrementalMptError::AccountProofMismatch(
                    ap.address,
                    H256::zero(),
                    prev_root,
                ));
            }
        }
    }

    // Verify storage proofs against each account's storage root.
    for sp in state.storage_proofs() {
        // Find the account's storage root.
        let account = state
            .account_proofs()
            .iter()
            .find(|ap| ap.address == sp.address);

        if let Some(ap) = account {
            let storage_root = ap.storage_root;
            if storage_root != *EMPTY_TRIE_HASH {
                let slot_path = keccak_hash(sp.slot.as_bytes()).to_vec();
                let storage_trie = build_trie_from_proofs(
                    storage_root,
                    vec![(slot_path.clone(), sp.storage_proof.clone())],
                )?;

                let computed_storage_root = storage_trie.hash_no_commit();
                if computed_storage_root != storage_root {
                    return Err(IncrementalMptError::StorageProofMismatch {
                        address: sp.address,
                        slot: sp.slot,
                    });
                }
            }
        }
    }

    Ok(())
}

/// Compute the new state root after applying all changes from the AppState.
///
/// This builds partial tries from the proofs, applies the dirty changes,
/// and recomputes the root hashes incrementally.
///
/// If there are no dirty accounts (e.g., empty blocks with zero transactions),
/// the previous state root is returned unchanged.
pub fn compute_new_state_root(state: &AppState) -> Result<H256, IncrementalMptError> {
    let prev_root = state.prev_state_root();

    // Short-circuit: no changes means the state root is unchanged.
    if state.dirty_accounts().peekable().peek().is_none() && state.account_proofs().is_empty() {
        return Ok(prev_root);
    }

    // 1. Build state trie from account proofs.
    let mut state_trie = build_trie_from_proofs(
        prev_root,
        state
            .account_proofs()
            .iter()
            .map(|ap| {
                let path = keccak_hash(ap.address.as_bytes()).to_vec();
                (path, ap.proof.clone())
            })
            .collect(),
    )?;

    // 2. For each dirty account, update its storage trie and then the state trie.
    for (address, account_state) in state.dirty_accounts() {
        let mut updated_account = *account_state;

        // If this account has dirty storage, update the storage trie.
        if let Some(dirty_slots) = state.dirty_storage().get(address) {
            let storage_root = account_state.storage_root;

            // Build storage trie from proofs for this account.
            let storage_proofs: Vec<_> = state
                .storage_proofs()
                .iter()
                .filter(|sp| sp.address == *address)
                .map(|sp| {
                    let slot_path = keccak_hash(sp.slot.as_bytes()).to_vec();
                    (slot_path, sp.storage_proof.clone())
                })
                .collect();

            let mut storage_trie = build_trie_from_proofs(storage_root, storage_proofs)?;

            // Apply dirty storage slot updates.
            for (slot, new_value) in dirty_slots {
                let slot_path = keccak_hash(slot.as_bytes()).to_vec();
                if new_value.is_zero() {
                    let _ = storage_trie
                        .remove(&slot_path)
                        .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;
                } else {
                    let value_rlp = new_value.encode_to_vec();
                    storage_trie
                        .insert(slot_path, value_rlp)
                        .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;
                }
            }

            // Recompute storage root.
            updated_account.storage_root = storage_trie.hash_no_commit();
        }

        // 3. Update account in state trie.
        let account_path = keccak_hash(address.as_bytes()).to_vec();
        let account_rlp = updated_account.encode_to_vec();
        state_trie
            .insert(account_path, account_rlp)
            .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;
    }

    // 4. Compute new state root.
    Ok(state_trie.hash_no_commit())
}

/// Build a partial trie from Merkle proof nodes.
///
/// The proof nodes are RLP-encoded trie nodes along the path from root
/// to a specific leaf. Multiple proofs can be combined to build a trie
/// that covers multiple paths.
///
/// Each proof consists of a lookup path and a list of RLP-encoded nodes
/// from root to leaf. The function decodes each node to determine its
/// type (Branch, Extension, Leaf) and computes the correct nibble path
/// position for storage in the DB, matching the trie's traversal logic.
fn build_trie_from_proofs(
    expected_root: H256,
    proofs: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
) -> Result<Trie, IncrementalMptError> {
    let db_map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let db = Arc::new(Mutex::new(db_map));
    let in_memory_db = InMemoryTrieDB::new(db.clone());

    for (path, proof_nodes) in &proofs {
        // Convert the lookup path to nibbles for tracking position.
        let mut nibble_path = Nibbles::from_bytes(path);

        for (i, node_rlp) in proof_nodes.iter().enumerate() {
            if i == 0 {
                // Root node is always stored at the default (empty) nibble path.
                in_memory_db
                    .put(Nibbles::default(), node_rlp.clone())
                    .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;
            } else {
                // Intermediate/leaf nodes: stored at the consumed nibble path.
                // `nibble_path.current()` returns the already-consumed portion.
                let db_key = nibble_path.current();
                in_memory_db
                    .put(db_key, node_rlp.clone())
                    .map_err(|e| IncrementalMptError::Trie(e.to_string()))?;
            }

            // Decode the node to determine how many nibbles it consumes,
            // so we can compute the correct DB key for the next node.
            let node = Node::decode(node_rlp)
                .map_err(|e| IncrementalMptError::RlpDecode(e.to_string()))?;
            match node {
                Node::Branch(_) => {
                    // Branch consumes one nibble (the choice index).
                    let _ = nibble_path.next_choice();
                }
                Node::Extension(ext) => {
                    // Extension consumes its prefix nibbles.
                    nibble_path.skip_prefix(&ext.prefix);
                }
                Node::Leaf(_) => {
                    // Leaf is the end of the path, no more nodes to store.
                    break;
                }
            }
        }
    }

    // Open trie with the expected root.
    let trie = Trie::open(Box::new(in_memory_db), expected_root);
    Ok(trie)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{H160, U256};

    use crate::common::app_types::AccountProof;

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

    /// Test that a state with no dirty changes produces the same root.
    #[test]
    fn unchanged_state_same_root() {
        // Build a real trie to get a valid root and proofs.
        let mut trie = Trie::empty_in_memory();
        let addr = test_address(1);
        let account = test_account(0, 1000);

        let path = keccak_hash(addr.as_bytes()).to_vec();
        trie.insert(path.clone(), account.encode_to_vec()).unwrap();
        let root = trie.hash_no_commit();

        // Get proof for this account.
        let proof = trie.get_proof(&path).unwrap();

        let account_proofs = vec![AccountProof {
            address: addr,
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
            proof,
        }];

        let state = AppState::from_proofs(root, account_proofs, vec![]);

        // No changes → same root.
        let new_root = compute_new_state_root(&state).unwrap();
        assert_eq!(new_root, root, "unchanged state should produce same root");
    }

    /// Test that modifying balance changes the root.
    #[test]
    fn balance_change_updates_root() {
        let mut trie = Trie::empty_in_memory();
        let addr = test_address(1);
        let account = test_account(0, 1000);

        let path = keccak_hash(addr.as_bytes()).to_vec();
        trie.insert(path.clone(), account.encode_to_vec()).unwrap();
        let root = trie.hash_no_commit();

        let proof = trie.get_proof(&path).unwrap();

        let account_proofs = vec![AccountProof {
            address: addr,
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
            proof,
        }];

        let mut state = AppState::from_proofs(root, account_proofs, vec![]);

        // Change balance.
        state.set_balance(addr, U256::from(2000)).unwrap();

        let new_root = compute_new_state_root(&state).unwrap();
        assert_ne!(
            new_root, root,
            "balance change should produce different root"
        );

        // Verify the new root matches a trie built from scratch with the updated value.
        let mut expected_trie = Trie::empty_in_memory();
        let updated_account = test_account(0, 2000);
        expected_trie
            .insert(path, updated_account.encode_to_vec())
            .unwrap();
        let expected_root = expected_trie.hash_no_commit();

        assert_eq!(
            new_root, expected_root,
            "incremental root should match full recomputation"
        );
    }

    /// Test that nonce increment changes the root correctly.
    #[test]
    fn nonce_increment_updates_root() {
        let mut trie = Trie::empty_in_memory();
        let addr = test_address(1);
        let account = test_account(5, 1000);

        let path = keccak_hash(addr.as_bytes()).to_vec();
        trie.insert(path.clone(), account.encode_to_vec()).unwrap();
        let root = trie.hash_no_commit();

        let proof = trie.get_proof(&path).unwrap();

        let account_proofs = vec![AccountProof {
            address: addr,
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
            proof,
        }];

        let mut state = AppState::from_proofs(root, account_proofs, vec![]);

        // Increment nonce.
        state.verify_and_increment_nonce(addr, 5).unwrap();

        let new_root = compute_new_state_root(&state).unwrap();

        // Verify against full recomputation.
        let mut expected_trie = Trie::empty_in_memory();
        let updated_account = test_account(6, 1000);
        expected_trie
            .insert(path, updated_account.encode_to_vec())
            .unwrap();
        let expected_root = expected_trie.hash_no_commit();

        assert_eq!(new_root, expected_root);
    }

    /// Test multi-account trie with branch nodes — the proof reconstruction
    /// must correctly store intermediate nodes at their nibble path positions.
    #[test]
    fn multi_account_unchanged_root() {
        let mut trie = Trie::empty_in_memory();

        // Insert 20 accounts to force branch nodes in the trie.
        let accounts: Vec<(Address, AccountState)> = (0u8..20)
            .map(|i| {
                (
                    test_address(i),
                    test_account(u64::from(i), 1000 + u64::from(i)),
                )
            })
            .collect();

        for (addr, account) in &accounts {
            let path = keccak_hash(addr.as_bytes()).to_vec();
            trie.insert(path, account.encode_to_vec()).unwrap();
        }
        let root = trie.hash_no_commit();

        // Get proofs for all accounts.
        let account_proofs: Vec<AccountProof> = accounts
            .iter()
            .map(|(addr, account)| {
                let path = keccak_hash(addr.as_bytes()).to_vec();
                let proof = trie.get_proof(&path).unwrap();
                AccountProof {
                    address: *addr,
                    nonce: account.nonce,
                    balance: account.balance,
                    storage_root: account.storage_root,
                    code_hash: account.code_hash,
                    proof,
                }
            })
            .collect();

        let state = AppState::from_proofs(root, account_proofs, vec![]);

        // Verify proofs.
        verify_state_proofs(&state).unwrap();

        // No changes → same root.
        let new_root = compute_new_state_root(&state).unwrap();
        assert_eq!(
            new_root, root,
            "multi-account unchanged state should produce same root"
        );
    }

    /// Test multi-account trie with balance update.
    #[test]
    fn multi_account_balance_update() {
        let mut trie = Trie::empty_in_memory();

        let accounts: Vec<(Address, AccountState)> = (0u8..10)
            .map(|i| (test_address(i), test_account(0, 1000 + u64::from(i))))
            .collect();

        for (addr, account) in &accounts {
            let path = keccak_hash(addr.as_bytes()).to_vec();
            trie.insert(path, account.encode_to_vec()).unwrap();
        }
        let root = trie.hash_no_commit();

        let account_proofs: Vec<AccountProof> = accounts
            .iter()
            .map(|(addr, account)| {
                let path = keccak_hash(addr.as_bytes()).to_vec();
                let proof = trie.get_proof(&path).unwrap();
                AccountProof {
                    address: *addr,
                    nonce: account.nonce,
                    balance: account.balance,
                    storage_root: account.storage_root,
                    code_hash: account.code_hash,
                    proof,
                }
            })
            .collect();

        let mut state = AppState::from_proofs(root, account_proofs, vec![]);

        // Modify first account's balance.
        let target = accounts[0].0;
        state.set_balance(target, U256::from(9999)).unwrap();

        let new_root = compute_new_state_root(&state).unwrap();
        assert_ne!(new_root, root, "balance change should alter root");

        // Verify against full recomputation.
        let mut expected_trie = Trie::empty_in_memory();
        for (i, (addr, account)) in accounts.iter().enumerate() {
            let mut updated = *account;
            if i == 0 {
                updated.balance = U256::from(9999);
            }
            let path = keccak_hash(addr.as_bytes()).to_vec();
            expected_trie.insert(path, updated.encode_to_vec()).unwrap();
        }
        let expected_root = expected_trie.hash_no_commit();
        assert_eq!(new_root, expected_root, "incremental root should match");
    }
}
