//! Unified read path for binary trie state.
//!
//! [`BinaryTrieWrapper`] provides account and storage reads that walk:
//! 1. The in-memory per-block layer cache (most recent, uncommitted state).
//! 2. The binary trie state (in-memory trie nodes from the last flush).
//! 3. Falls through to FKV on disk for committed blocks not in either.
//!
//! This eliminates the need for per-function ad-hoc layer cache checks.

use ethrex_binary_trie::{
    key_mapping::{
        get_tree_key_for_basic_data, get_tree_key_for_code_hash, get_tree_key_for_storage_slot,
        unpack_basic_data,
    },
    layer_cache::BinaryTrieLayerCache,
    state::BinaryTrieState,
};
use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::AccountState,
};

/// Provides unified account and storage reads across binary trie layers and
/// the committed trie state.
///
/// Reading order:
/// 1. Layer cache (per-block leaf diffs, newest first).
/// 2. `BinaryTrieState` in-memory trie (covers data committed to trie nodes
///    but not yet flushed to FKV).
/// 3. Caller falls through to FKV if this returns `None`.
pub struct BinaryTrieWrapper<'a> {
    pub trie_root: [u8; 32],
    pub layer_cache: &'a BinaryTrieLayerCache,
    pub trie_state: &'a BinaryTrieState,
}

impl<'a> BinaryTrieWrapper<'a> {
    /// Look up a leaf by its 32-byte tree key.
    ///
    /// Returns:
    /// - `Some(Some(value))` — leaf found with this value.
    /// - `Some(None)` — leaf was explicitly deleted.
    /// - `None` — not found in any in-memory layer; caller should check FKV.
    pub fn get_leaf(&self, tree_key: &[u8; 32]) -> Option<Option<[u8; 32]>> {
        // 1. Check per-block layer cache.
        if let Some(result) = self.layer_cache.get(self.trie_root, tree_key) {
            return Some(result);
        }

        // 2. Check committed trie state (trie nodes loaded from disk at flush).
        self.trie_state.trie_get(*tree_key).map(Some)
    }

    /// Read account state for the given address.
    ///
    /// Returns `None` if the account does not exist in any in-memory layer.
    /// The caller should fall through to FKV in that case.
    pub fn get_account_state(&self, address: &Address) -> Option<Option<AccountState>> {
        let basic_data_key = get_tree_key_for_basic_data(address);
        match self.get_leaf(&basic_data_key) {
            Some(None) => {
                // Account explicitly deleted.
                Some(None)
            }
            Some(Some(basic_data)) => {
                let (_version, _code_size, nonce, balance) = unpack_basic_data(&basic_data);

                let code_hash_key = get_tree_key_for_code_hash(address);
                let code_hash = match self.get_leaf(&code_hash_key) {
                    Some(Some(hash)) => H256(hash),
                    Some(None) | None => *EMPTY_KECCACK_HASH,
                };

                // Synthesize storage_root: use the authoritative has_storage_keys check.
                // The trie state tracks all storage keys cumulatively, so this is correct
                // for determining whether any storage exists.
                let storage_root = if self.trie_state.has_storage_keys(address) {
                    H256::from_low_u64_be(1)
                } else {
                    *EMPTY_TRIE_HASH
                };

                Some(Some(AccountState {
                    nonce,
                    balance,
                    storage_root,
                    code_hash,
                }))
            }
            None => {
                // Not found in any in-memory layer — caller falls through to FKV.
                None
            }
        }
    }

    /// Read a storage slot value.
    ///
    /// Returns:
    /// - `Some(Some(value))` — slot found with this value.
    /// - `Some(None)` — slot was deleted.
    /// - `None` — not found in any in-memory layer; caller should check FKV.
    pub fn get_storage_slot(&self, address: &Address, storage_key: H256) -> Option<Option<U256>> {
        let key_u256 = U256::from_big_endian(storage_key.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(address, key_u256);
        match self.get_leaf(&tree_key) {
            Some(Some(value)) => Some(Some(U256::from_big_endian(&value))),
            Some(None) => Some(None),
            None => None,
        }
    }
}
