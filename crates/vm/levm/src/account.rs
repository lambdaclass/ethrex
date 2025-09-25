use ethrex_common::{H256, utils::keccak};
use ethrex_common::{
    U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, GenesisAccount},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Similar to `Account` struct but suited for LEVM implementation.
/// Difference is this doesn't have code and it contains an additional `status` field for decision-making.
/// The code is stored in the `GeneralizedDatabase` and can be accessed with its hash.\
/// **Some advantages:**
/// - We'll fetch the code only if we need to, this means less accesses to the database.
/// - If there's duplicate code between accounts (which is pretty common) we'll store it in memory only once.
/// - We'll be able to make better decisions without relying on external structures, based on the current status of an Account. e.g. If it was untouched we skip processing it when calculating Account Updates, or if the account has been destroyed and re-created with same address we know that the storage on the Database is not valid and we shouldn't access it, etc.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevmAccount {
    pub info: AccountInfo,
    pub storage: BTreeMap<H256, U256>,
    /// Current status of the account.
    pub status: AccountStatus,
}

impl From<AccountInfo> for LevmAccount {
    fn from(info: AccountInfo) -> Self {
        LevmAccount {
            info,
            storage: BTreeMap::new(),
            status: AccountStatus::Unmodified,
        }
    }
}

impl From<GenesisAccount> for LevmAccount {
    fn from(genesis: GenesisAccount) -> Self {
        let storage = genesis
            .storage
            .into_iter()
            .map(|(key, value)| (H256::from(key.to_big_endian()), value))
            .collect();

        LevmAccount {
            info: AccountInfo {
                balance: genesis.balance,
                nonce: genesis.nonce,
                code_hash: keccak(&genesis.code),
            },
            storage,
            status: AccountStatus::Unmodified,
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

    pub fn has_code_or_nonce(&self) -> bool {
        self.has_code() || self.has_nonce()
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
