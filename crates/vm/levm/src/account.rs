use ethrex_common::{U256, types::AccountInfo};
use keccak_hash::H256;
use std::collections::BTreeMap;

/// Similar to Account but suited for LEVM implementation.
/// Difference is this doesn't have code and it contains an additional `status` field for decision-making
/// The code is stored in the GeneralizedDatabase and can be accessed with the hash.
pub struct LevmAccount {
    info: AccountInfo,
    storage: BTreeMap<H256, U256>,
    /// Current status of the account.
    status: AccountStatus,
}

pub enum AccountStatus {
    Unmodified,
    Modified,
    /// Contract executed a SELFDESTRUCT
    Destroyed,
    /// Contract created via external transaction or CREATE/CREATE2
    Created,
    /// Contract has been destroyed and then re-created, usually with CREATE2
    /// This is a particular state because we'll still have in the Database the storage (trie) values but they are actually invalid.
    DestroyedCreated,
}
