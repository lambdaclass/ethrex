//! `apply_bal`: apply a single `BlockAccessList` diff to a state trie.
//!
//! # Account destruction encoding
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
//! and avoids any spec ambiguity.

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
    api::tables::{ACCOUNT_CODE_METADATA, ACCOUNT_CODES, ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES},
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
/// Callers must only invoke this function when the block is post-Amsterdam
/// (i.e. `header.block_access_list_hash.is_some()` or equivalent fork check).
pub fn apply_bal(
    store: &Store,
    parent_state_root: H256,
    bal: &BlockAccessList,
    block_header: &BlockHeader,
) -> Result<H256, SyncError> {
    // Empty BAL: state root unchanged.
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
                let code = Code::from_bytecode_unchecked(last_cc.new_code.clone(), code_hash);
                store_code_sync(store, code)?;
                account_state.code_hash = code_hash;
            }
        }

        // Step 2e: apply storage changes — post_value is authoritative.
        // Pre-state coverage: missing slots are treated as zero (no error).
        if !account_changes.storage_changes.is_empty() {
            // open_storage_trie's second arg (state_root) is used by TrieLayerCache
            // as the entry point to the in-memory layer chain. During BAL replay,
            // storage nodes are written directly to the backend via write_batch and
            // never entered into the cache, so the cache lookup always falls through
            // to disk regardless of which root is passed.
            let mut storage_trie = store.open_storage_trie(
                hashed_addr,
                parent_state_root,
                account_state.storage_root,
            )?;

            for slot_change in &account_changes.storage_changes {
                // u256_to_h256: slot is a U256, convert to H256 big-endian.
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

    // Per-block state root check (§68 / EIP-8189).
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
    let hash_key_bytes = code.hash.0.to_vec();
    let mut buf = Vec::new();
    code.bytecode.encode(&mut buf);
    code.jump_targets.encode(&mut buf);
    let metadata = (code.bytecode.len() as u64).to_be_bytes().to_vec();

    store.write(ACCOUNT_CODES, hash_key_bytes.clone(), buf)?;
    store.write(ACCOUNT_CODE_METADATA, hash_key_bytes, metadata)?;
    Ok(())
}
