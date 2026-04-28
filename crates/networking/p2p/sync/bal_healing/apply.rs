//! `apply_bal`: apply a single `BlockAccessList` diff to a state trie.
//!
//! # Account destruction encoding (Task 0.5 resolution)
//!
//! EIP-7928 does **not** define an explicit destruction marker on `AccountChanges`.
//! The struct carries `balance_changes`, `nonce_changes`, `code_changes`,
//! `storage_changes`, and `storage_reads` — no `destroyed` field exists.
//!
//! Rule adopted for BAL replay (implicit-empty):
//!   An account is considered destroyed after applying all changes if and only if
//!   `balance == 0 AND nonce == 0 AND code_hash == EMPTY_KECCAK_HASH AND storage_root == EMPTY_TRIE_HASH`.
//!   In that case the account node is deleted from the state trie rather than stored.
//!
//! This matches EVM account deletion semantics (EIP-161 empty-account removal)
//! and avoids any spec ambiguity. Phase 6 Task 6.2 step 2g is implemented against this rule.

use ethrex_common::{
    H256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, BlockHeader, Code, block_access_list::BlockAccessList},
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{
    Store,
    api::tables::{ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES},
    apply_prefix, hash_address, hash_key,
};
use ethrex_trie::EMPTY_TRIE_HASH as TRIE_EMPTY;

use crate::sync::SyncError;

/// Apply a single `BlockAccessList` to the state trie rooted at `parent_state_root`.
///
/// Returns the new state root after applying all account/storage changes from `bal`.
///
/// Pre-state coverage rule: missing accounts are treated as freshly created (default
/// `AccountState`). Missing storage slots are treated as zero.
///
/// Code changes are written to the code store immediately.
///
/// # Fork gate
/// Callers must only invoke this function when
/// `chain_config.is_amsterdam_activated(block_header.timestamp)` is true.
pub fn apply_bal(
    store: &Store,
    parent_state_root: H256,
    bal: &BlockAccessList,
    block_header: &BlockHeader,
) -> Result<H256, SyncError> {
    // Empty BAL: state root unchanged (Task 7.5).
    if bal.is_empty() {
        return Ok(parent_state_root);
    }

    let mut state_trie = store.open_state_trie(parent_state_root)?;
    // Accumulate storage trie nodes so we can persist them after the state root is computed.
    let mut storage_trie_batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

    for account_changes in bal.accounts() {
        let hashed_addr_bytes = hash_address(&account_changes.address);
        let hashed_addr = H256::from_slice(&hashed_addr_bytes);

        // Step 2a: read existing account (or fresh default).
        let mut account_state: AccountState = match state_trie.get(&hashed_addr_bytes)? {
            Some(encoded) => AccountState::decode(&encoded)?,
            None => AccountState::default(),
        };

        // Step 2b: apply balance changes — final entry wins.
        if let Some(last_bc) = account_changes.balance_changes.last() {
            account_state.balance = last_bc.post_balance;
        }

        // Step 2c: apply nonce changes — final entry wins.
        if let Some(last_nc) = account_changes.nonce_changes.last() {
            account_state.nonce = last_nc.post_nonce;
        }

        // Step 2d: apply code changes — final entry wins.
        if let Some(last_cc) = account_changes.code_changes.last() {
            if last_cc.new_code.is_empty() {
                // Delegation clear (EIP-7702) or code removal: set code_hash to empty.
                account_state.code_hash = *EMPTY_KECCACK_HASH;
            } else {
                let code_hash = keccak(&last_cc.new_code);
                // Write code to backing store.
                let code = Code::from_bytecode_unchecked(last_cc.new_code.clone(), code_hash);
                store_code_sync(store, code)?;
                account_state.code_hash = code_hash;
            }
        }

        // Step 2e: apply storage changes — post_value is authoritative.
        // Pre-state coverage: missing slots are treated as zero (no error).
        if !account_changes.storage_changes.is_empty() {
            // `open_storage_trie`'s second argument (`state_root`) is used by
            // `TrieLayerCache` as the entry point to the in-memory layer chain.
            // During BAL replay, storage nodes are written directly to the backend
            // via `write_batch(STORAGE_TRIE_NODES, …)` and never entered into the
            // cache, so the cache lookup always falls through to disk regardless of
            // which root is passed. Using `parent_state_root` here is therefore
            // correct: all storage reads in this call go to the backend, and the
            // written nodes are collected and flushed at the end of `apply_bal`.
            let mut storage_trie = store.open_storage_trie(
                hashed_addr,
                parent_state_root,
                account_state.storage_root,
            )?;

            for slot_change in &account_changes.storage_changes {
                let hashed_slot = hash_key(&H256::from(slot_change.slot.to_big_endian()));
                // Take the final post_value for this slot.
                if let Some(last_change) = slot_change.slot_changes.last() {
                    if last_change.post_value.is_zero() {
                        // Slot deletion: zero post_value removes the slot.
                        storage_trie.remove(&hashed_slot)?;
                    } else {
                        storage_trie.insert(hashed_slot, last_change.post_value.encode_to_vec())?;
                    }
                }
            }

            let (new_storage_root, storage_nodes) =
                storage_trie.collect_changes_since_last_hash(&NativeCrypto);
            account_state.storage_root = new_storage_root;

            // Accumulate storage nodes (prefixed by account hash) for later backend write.
            for (path, rlp) in storage_nodes {
                let key = apply_prefix(Some(hashed_addr), path).into_vec();
                storage_trie_batch.push((key, rlp));
            }
        }

        // Step 2f: storage_reads are skipped — we only apply post-values.

        // Step 2g: destruction check (implicit-empty rule).
        let is_destroyed = account_state.balance.is_zero()
            && account_state.nonce == 0
            && account_state.code_hash == *EMPTY_KECCACK_HASH
            && (account_state.storage_root == *EMPTY_TRIE_HASH
                || account_state.storage_root == *TRIE_EMPTY);

        if is_destroyed {
            state_trie.remove(&hashed_addr_bytes)?;
        } else {
            state_trie.insert(hashed_addr_bytes, account_state.encode_to_vec())?;
        }
    }

    let (new_state_root, state_nodes) = state_trie.collect_changes_since_last_hash(&NativeCrypto);

    // Per-block state root check.
    if new_state_root != block_header.state_root {
        return Err(SyncError::StateRootMismatch(
            block_header.state_root,
            new_state_root,
        ));
    }

    // Persist state and storage trie nodes to the backend so subsequent reads succeed.
    let state_trie_batch: Vec<(Vec<u8>, Vec<u8>)> = state_nodes
        .into_iter()
        .map(|(path, rlp)| (apply_prefix(None, path).into_vec(), rlp))
        .collect();
    store.write_batch(ACCOUNT_TRIE_NODES, state_trie_batch)?;
    if !storage_trie_batch.is_empty() {
        store.write_batch(STORAGE_TRIE_NODES, storage_trie_batch)?;
    }

    Ok(new_state_root)
}

/// Write a `Code` entry to the store synchronously.
fn store_code_sync(store: &Store, code: Code) -> Result<(), SyncError> {
    use ethrex_rlp::encode::RLPEncode;
    use ethrex_storage::api::tables::{ACCOUNT_CODE_METADATA, ACCOUNT_CODES};

    let hash_key_bytes = code.hash.as_bytes().to_vec();
    let mut buf = Vec::new();
    code.bytecode.encode(&mut buf);
    code.jump_targets.encode(&mut buf);

    let metadata = (code.bytecode.len() as u64).to_be_bytes().to_vec();

    store.write(ACCOUNT_CODES, hash_key_bytes.clone(), buf)?;
    store.write(ACCOUNT_CODE_METADATA, hash_key_bytes, metadata)?;
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ethrex_common::{
        Address, H256, U256,
        types::block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange, SlotChange,
            StorageChange,
        },
    };
    use ethrex_storage::{EngineType, Store, api::tables::STORAGE_TRIE_NODES};

    fn empty_store() -> Store {
        Store::new("test", EngineType::InMemory).expect("failed to create in-memory store")
    }

    /// Build a minimal block header with the given state root.
    fn header_with_root(state_root: H256) -> BlockHeader {
        let mut h = BlockHeader::default();
        h.state_root = state_root;
        h
    }

    /// Build pre-state with a single account and persist the trie nodes to the backend.
    ///
    /// Returns the state root after inserting the account. Uses `open_direct_state_trie` so
    /// nodes are computed via the backend path (no layer cache), then writes them via
    /// `write_batch` so subsequent `open_state_trie` calls can find them.
    fn insert_account_into_store(store: &Store, addr: Address, acct: &AccountState) -> H256 {
        let hashed = hash_address(&addr);
        let mut trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        trie.insert(hashed, acct.encode_to_vec()).unwrap();
        let (root, nodes) = trie.collect_changes_since_last_hash(&NativeCrypto);
        let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
            .into_iter()
            .map(|(path, rlp)| (apply_prefix(None, path).into_vec(), rlp))
            .collect();
        store.write_batch(ACCOUNT_TRIE_NODES, batch).unwrap();
        root
    }

    /// Build a storage trie with a single slot and persist its nodes to the backend.
    ///
    /// Returns the storage root.
    fn insert_storage_slot(
        store: &Store,
        addr_hash: H256,
        slot_key: Vec<u8>,
        value: Vec<u8>,
    ) -> H256 {
        let mut trie = store
            .open_direct_storage_trie(addr_hash, *EMPTY_TRIE_HASH)
            .unwrap();
        trie.insert(slot_key, value).unwrap();
        let (root, nodes) = trie.collect_changes_since_last_hash(&NativeCrypto);
        let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
            .into_iter()
            .map(|(path, rlp)| (apply_prefix(Some(addr_hash), path).into_vec(), rlp))
            .collect();
        store.write_batch(STORAGE_TRIE_NODES, batch).unwrap();
        root
    }

    // Task 6.3: account creation — address absent in pre-state; BAL carries changes.
    #[test]
    fn apply_bal_account_creation() {
        let store = empty_store();
        let empty_root = *EMPTY_TRIE_HASH;

        let addr = Address::from([0x01u8; 20]);
        let mut changes = AccountChanges::new(addr);
        changes.add_balance_change(BalanceChange::new(0, U256::from(100u64)));
        changes.add_nonce_change(NonceChange::new(1, 1));

        let mut bal = BlockAccessList::new();
        bal.add_account_changes(changes);

        // Compute expected state root after apply (using direct trie, no persistence needed
        // — only the hash is used to construct the header).
        let hashed = hash_address(&addr);
        let mut expected_acct = AccountState::default();
        expected_acct.balance = U256::from(100u64);
        expected_acct.nonce = 1;
        let mut expected_trie = store.open_direct_state_trie(empty_root).unwrap();
        expected_trie
            .insert(hashed, expected_acct.encode_to_vec())
            .unwrap();
        let (expected_root, _) = expected_trie.collect_changes_since_last_hash(&NativeCrypto);

        let header = header_with_root(expected_root);
        let new_root = apply_bal(&store, empty_root, &bal, &header).unwrap();
        assert_eq!(
            new_root, expected_root,
            "creation should produce correct root"
        );

        // apply_bal persists nodes; verify account is readable from the resulting root.
        let trie_after = store.open_state_trie(new_root).unwrap();
        let encoded = trie_after.get(&hash_address(&addr)).unwrap().unwrap();
        let acct = AccountState::decode(&encoded).unwrap();
        assert_eq!(acct.balance, U256::from(100u64));
        assert_eq!(acct.nonce, 1);
    }

    // Task 6.4: account destruction — post-state all zeros → account removed.
    #[test]
    fn apply_bal_account_destruction() {
        let store = empty_store();
        let addr = Address::from([0x02u8; 20]);

        // Pre-insert an account and persist to backend.
        let mut pre_acct = AccountState::default();
        pre_acct.balance = U256::from(500u64);
        pre_acct.nonce = 3;
        let pre_root = insert_account_into_store(&store, addr, &pre_acct);

        // BAL: zero out balance and nonce → implicit destruction.
        let mut changes = AccountChanges::new(addr);
        changes.add_balance_change(BalanceChange::new(0, U256::zero()));
        changes.add_nonce_change(NonceChange::new(1, 0));
        let mut bal = BlockAccessList::new();
        bal.add_account_changes(changes);

        // Expected: empty trie (account gone).
        let header = header_with_root(*EMPTY_TRIE_HASH);
        let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
        assert_eq!(
            new_root, *EMPTY_TRIE_HASH,
            "destroyed account should yield empty root"
        );

        let trie_after = store.open_state_trie(new_root).unwrap();
        assert!(
            trie_after.get(&hash_address(&addr)).unwrap().is_none(),
            "account should be absent after destruction"
        );
    }

    // Task 6.5: storage slot deletion — zero post_value removes slot.
    #[test]
    fn apply_bal_storage_slot_deletion() {
        let store = empty_store();
        let addr = Address::from([0x03u8; 20]);
        let slot = U256::from(42u64);

        let hashed_addr = hash_address(&addr);
        let hashed_addr_h256 = H256::from_slice(&hashed_addr);
        let slot_key = hash_key(&H256::from(slot.to_big_endian()));

        // Pre-insert storage slot and persist.
        let storage_root = insert_storage_slot(
            &store,
            hashed_addr_h256,
            slot_key.clone(),
            U256::from(99u64).encode_to_vec(),
        );

        // Pre-insert account referencing the storage root and persist.
        let mut pre_acct = AccountState::default();
        pre_acct.balance = U256::from(1u64); // keep alive
        pre_acct.storage_root = storage_root;

        let mut pre_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        pre_state_trie
            .insert(hashed_addr.clone(), pre_acct.encode_to_vec())
            .unwrap();
        let (pre_root, nodes) = pre_state_trie.collect_changes_since_last_hash(&NativeCrypto);
        let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
            .into_iter()
            .map(|(path, rlp)| (apply_prefix(None, path).into_vec(), rlp))
            .collect();
        store.write_batch(ACCOUNT_TRIE_NODES, batch).unwrap();

        // BAL: delete the slot (post_value = 0).
        let mut slot_change = SlotChange::new(slot);
        slot_change.add_change(StorageChange::new(0, U256::zero()));
        let mut changes = AccountChanges::new(addr);
        changes.add_storage_change(slot_change);
        let mut bal = BlockAccessList::new();
        bal.add_account_changes(changes);

        // Build expected root: storage trie empty → storage_root = EMPTY, account keeps balance.
        // Deletion of the sole slot yields empty storage root.
        let mut expected_acct = pre_acct;
        expected_acct.storage_root = *EMPTY_TRIE_HASH;
        let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        expected_state_trie
            .insert(hashed_addr, expected_acct.encode_to_vec())
            .unwrap();
        let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

        let header = header_with_root(expected_root);
        let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
        assert_eq!(
            new_root, expected_root,
            "slot deletion should produce correct root"
        );
    }

    // Task 6.6: code deployment — bytecode written under keccak(code); code_hash updated.
    #[test]
    fn apply_bal_code_deployment() {
        let store = empty_store();
        let addr = Address::from([0x04u8; 20]);
        let bytecode: bytes::Bytes = bytes::Bytes::from(vec![0x60, 0x00, 0x56]); // PUSH1 0 JUMP
        let code_hash = keccak(&bytecode);

        let mut changes = AccountChanges::new(addr);
        changes.add_balance_change(BalanceChange::new(0, U256::from(1u64)));
        changes.add_code_change(CodeChange::new(0, bytecode.clone()));
        let mut bal = BlockAccessList::new();
        bal.add_account_changes(changes);

        // Build expected root using direct trie (hash only, not persisted).
        let hashed = hash_address(&addr);
        let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        let mut expected_acct = AccountState::default();
        expected_acct.balance = U256::from(1u64);
        expected_acct.code_hash = code_hash;
        expected_state_trie
            .insert(hashed, expected_acct.encode_to_vec())
            .unwrap();
        let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

        let header = header_with_root(expected_root);
        let new_root = apply_bal(&store, *EMPTY_TRIE_HASH, &bal, &header).unwrap();
        assert_eq!(
            new_root, expected_root,
            "code deploy should produce correct root"
        );

        // Verify code is stored under the correct hash.
        let stored_code = store.get_account_code(code_hash).unwrap();
        assert!(stored_code.is_some(), "code should be stored in the store");
        assert_eq!(stored_code.unwrap().bytecode, bytecode);
    }

    // Task 6.7: EIP-7702 delegation clear — code_hash becomes EMPTY_KECCAK_HASH.
    #[test]
    fn apply_bal_delegation_clear() {
        let store = empty_store();
        let addr = Address::from([0x05u8; 20]);
        let old_code = bytes::Bytes::from(vec![0xEF, 0x01, 0x02]); // some delegation bytecode
        let old_code_hash = keccak(&old_code);

        // Pre-insert account with non-empty code and persist to backend.
        let mut pre_acct = AccountState::default();
        pre_acct.balance = U256::from(50u64);
        pre_acct.nonce = 2;
        pre_acct.code_hash = old_code_hash;
        let pre_root = insert_account_into_store(&store, addr, &pre_acct);

        // BAL: empty code_change clears the delegation.
        let mut changes = AccountChanges::new(addr);
        changes.add_code_change(CodeChange::new(0, bytes::Bytes::new()));
        let mut bal = BlockAccessList::new();
        bal.add_account_changes(changes);

        // Build expected root: same account but code_hash → EMPTY_KECCACK_HASH.
        let mut expected_acct = pre_acct;
        expected_acct.code_hash = *EMPTY_KECCACK_HASH;
        let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
        expected_state_trie
            .insert(hash_address(&addr), expected_acct.encode_to_vec())
            .unwrap();
        let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

        let header = header_with_root(expected_root);
        let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
        assert_eq!(
            new_root, expected_root,
            "delegation clear should produce correct root"
        );

        // apply_bal persists nodes; verify readable.
        let trie_after = store.open_state_trie(new_root).unwrap();
        let encoded = trie_after.get(&hash_address(&addr)).unwrap().unwrap();
        let acct = AccountState::decode(&encoded).unwrap();
        assert_eq!(
            acct.code_hash, *EMPTY_KECCACK_HASH,
            "code_hash should be EMPTY_KECCAK after delegation clear"
        );
    }
}
