use crate::{
    Address, H256, U256,
    types::{AccountInfo, Code},
};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountUpdate {
    pub address: Address,
    /// Pre-computed keccak256 hash of the address for merkleization.
    /// This allows the executor thread to compute hashes in parallel with
    /// the merkleizer, avoiding hashing overhead on the critical merkle path.
    #[serde(skip)]
    pub hashed_address: Option<H256>,
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

    /// Creates new empty update with pre-computed hashed address
    pub fn new_with_hash(address: Address, hashed_address: H256) -> AccountUpdate {
        AccountUpdate {
            address,
            hashed_address: Some(hashed_address),
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

    /// Creates new update representing an account removal with pre-computed hash
    pub fn removed_with_hash(address: Address, hashed_address: H256) -> AccountUpdate {
        AccountUpdate {
            address,
            hashed_address: Some(hashed_address),
            removed: true,
            ..Default::default()
        }
    }

    pub fn merge(&mut self, other: AccountUpdate) {
        self.removed = other.removed;
        self.removed_storage |= other.removed_storage;
        // Keep our hashed_address if we have one, otherwise take theirs
        if self.hashed_address.is_none() {
            self.hashed_address = other.hashed_address;
        }
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
