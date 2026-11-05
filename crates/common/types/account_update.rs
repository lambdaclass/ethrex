use crate::seeded_hash::SeededHashMap;
use crate::{
    Address, H256, U256,
    types::{AccountInfo, Code},
};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountUpdate {
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,
    pub code: Option<Code>,
    /// Seeded: contracts choose these slot keys.
    pub added_storage: SeededHashMap<H256, U256>,
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

/// One account's change in a build, with the parent values it replaces.
/// A rebase uses the parent values to undo a change a newer build no longer makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDiff {
    /// Exactly what `get_state_transitions` emits for this account.
    pub update: AccountUpdate,
    /// The account's info in the parent state.
    pub parent_info: AccountInfo,
    /// Parent value of every slot in `update.added_storage`.
    pub parent_storage: SeededHashMap<H256, U256>,
}

/// A build's full change over its parent state.
pub type StateDiff = SeededHashMap<Address, AccountDiff>;
