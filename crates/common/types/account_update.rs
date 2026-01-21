use crate::{
    Address, H256, U256,
    types::{AccountInfo, Code},
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

    /// Merges multiple AccountUpdates for the same account into one.
    ///
    /// Pre-allocates storage capacity based on the iterator size hint
    /// to minimize reallocations during merging.
    ///
    /// Returns `None` if the iterator is empty.
    pub fn merge_batch(updates: impl IntoIterator<Item = Self>) -> Option<Self> {
        let mut iter = updates.into_iter();
        let mut result = iter.next()?;

        // Pre-allocate based on iterator size hint
        let (lower, upper) = iter.size_hint();
        // Estimate ~4 storage entries per update on average
        result
            .added_storage
            .reserve(upper.unwrap_or(lower).saturating_mul(4));

        for update in iter {
            result.merge(update);
        }
        Some(result)
    }
}
