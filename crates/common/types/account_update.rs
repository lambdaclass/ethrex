use crate::{
    Address, H256, U256,
    types::{AccountInfo, Code},
    utils::keccak,
};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountUpdate {
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,
    pub code: Option<Code>,
    pub added_storage: FxHashMap<H256, U256>,
    /// If account was destroyed and then modified we need this for removing its storage but not the entire account.
    pub removed_storage: bool,
    // Matches TODO in code
    // removed_storage_keys: Vec<H256>,
}

impl AccountUpdate {
    /// Creates new empty update for the given account
    pub fn new(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            ..Default::default()
        }
    }

    /// Creates new update representing an account removal
    pub fn removed(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            removed: true,
            ..Default::default()
        }
    }

    pub fn merge(&mut self, other: AccountUpdate) {
        self.removed = other.removed;
        self.removed_storage |= other.removed_storage;
        if let Some(info) = other.info {
            self.info = Some(info);
        }
        if let Some(code) = other.code {
            self.code = Some(code);
        }
        for (key, value) in other.added_storage {
            self.added_storage.insert(key, value);
        }
    }
}

/// An `AccountUpdate` with pre-computed keccak256 hashes for the address
/// and all storage keys. Used by the hashing intermediary thread to
/// offload keccak work from the merkleizer.
#[derive(Debug, Clone)]
pub struct HashedAccountUpdate {
    pub hashed_address: H256,
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,
    pub code: Option<Code>,
    /// Storage entries with pre-hashed keys: (hashed_key, value).
    pub added_storage: Vec<(H256, U256)>,
    pub removed_storage: bool,
}

impl HashedAccountUpdate {
    /// Hash an `AccountUpdate`'s address and storage keys.
    pub fn from_update(update: AccountUpdate) -> Self {
        let hashed_address = keccak(update.address);
        let added_storage = update
            .added_storage
            .into_iter()
            .map(|(key, value)| (keccak(key), value))
            .collect();
        Self {
            hashed_address,
            address: update.address,
            removed: update.removed,
            info: update.info,
            code: update.code,
            added_storage,
            removed_storage: update.removed_storage,
        }
    }
}
