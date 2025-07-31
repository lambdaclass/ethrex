use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_common::{U256, types::AccountInfo};
use keccak_hash::H256;

/// Similar to Account but suited for LEVM implementation.
/// Difference is that code is an Option and it contains an additional `status` field for decision-making
pub struct LevmAccount {
    info: AccountInfo,
    /// If `None` it means it hasn't been fetched, empty code is Some(Bytes::new())
    /// Code will only be fetched when necessary, because sometimes we don't need it. e.g. When processing withdrawals
    code: Option<Bytes>,
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
