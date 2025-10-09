use ethrex_common::H256;
use ethrex_common::constants::EMPTY_TRIE_HASH;
use ethrex_common::types::{AccountState, GenesisAccount};
use ethrex_common::utils::keccak;
use ethrex_common::{U256, constants::EMPTY_KECCACK_HASH, types::AccountInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Similar to `Account` struct but suited for LEVM implementation.
/// Difference is this doesn't have code and it contains an additional `status` field for decision-making.
/// The code is stored in the `GeneralizedDatabase` and can be accessed with its hash.\
/// **Some advantages:**
/// - We'll fetch the code only if we need to, this means less accesses to the database.
/// - If there's duplicate code between accounts (which is pretty common) we'll store it in memory only once.
/// - We'll be able to make better decisions without relying on external structures, based on the current status of an Account. e.g. If it was untouched we skip processing it when calculating Account Updates, or if the account has been destroyed and re-created with same address we know that the storage on the Database is not valid and we shouldn't access it, etc.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevmAccount {
    pub info: AccountInfo,
    pub storage: BTreeMap<H256, U256>,
    /// If true it means that attempting to create an account with this address it would at least collide because of storage.
    /// We just care about this kind of collision if the account doesn't have code or nonce. Otherwise its value doesn't matter.
    /// For more information see EIP-7610: https://eips.ethereum.org/EIPS/eip-7610
    /// Warning: This attribute should only be used for handling create collisions as it's not necessary appropriate for every scenario. Read the caveat below.
    ///
    /// How this works:
    /// - When getting an account from the DB this is set to true if the account has non-empty storage root.
    /// - Upon destruction of an account this is set to false because storage is emptied for sure.
    ///
    /// **Important Caveat**
    /// This only works for accounts of these characteristics that have been created in the past, we consider that accounts with storage
    /// but no nonce or code cannot be created anymore, otherwise the fix would need to be more complex because we should keep track of the
    /// storage root of an account during execution instead of just keeping track of it when fetching it from the Database or updating it when
    /// destroying it. The EIP that adds to the spec this check did it because there are 28 accounts with these characteristics already deployed
    /// in mainnet (back when they were deployed with nonce 0), but they cannot be created intentionally anymore.
    pub has_storage: bool,
    /// Current status of the account.
    /// Note that currently we are not using AccountStatus for any important checks but we should probably start using it soon.
    /// Some examples include: Knowing if an account was destroyed and re-created afterwards, or maybe if we use the Unmodified variant correctly we can skip processing updates of the account.
    pub status: AccountStatus,
}

// This is used only in state_v2 runner, storage is already fully filled in the genesis account.
impl From<GenesisAccount> for LevmAccount {
    fn from(genesis: GenesisAccount) -> Self {
        let storage: BTreeMap<H256, U256> = genesis
            .storage
            .into_iter()
            .map(|(key, value)| (H256::from(key.to_big_endian()), value))
            .collect();

        LevmAccount {
            info: AccountInfo {
                code_hash: keccak(genesis.code),
                balance: genesis.balance,
                nonce: genesis.nonce,
            },
            has_storage: !storage.is_empty(),
            storage,
            status: AccountStatus::Unmodified,
        }
    }
}
impl From<AccountState> for LevmAccount {
    fn from(state: AccountState) -> Self {
        LevmAccount {
            info: AccountInfo {
                code_hash: state.code_hash,
                balance: state.balance,
                nonce: state.nonce,
            },
            storage: BTreeMap::new(),
            status: AccountStatus::Unmodified,
            has_storage: state.storage_root != *EMPTY_TRIE_HASH,
        }
    }
}

impl LevmAccount {
    pub fn has_nonce(&self) -> bool {
        self.info.nonce != 0
    }

    pub fn has_code(&self) -> bool {
        self.info.code_hash != *EMPTY_KECCACK_HASH
    }

    pub fn create_would_collide(&self) -> bool {
        self.has_code() || self.has_nonce() || self.has_storage
    }

    pub fn is_empty(&self) -> bool {
        self.info.is_empty()
    }

    /// Updates the account status.
    pub fn update_status(&mut self, status: AccountStatus) {
        self.status = status;
    }

    /// Checks if the account is unmodified.
    pub fn is_unmodified(&self) -> bool {
        matches!(self.status, AccountStatus::Unmodified)
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    #[default]
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
