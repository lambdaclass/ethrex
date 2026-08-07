use ethrex_common::H256;
use ethrex_common::types::{AccountState, GenesisAccount};
use ethrex_common::utils::keccak;
use ethrex_common::{U256, constants::EMPTY_KECCAK_HASH, types::AccountInfo};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

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
    pub storage: FxHashMap<H256, U256>,
    /// If true it means that attempting to create an account with this address it would at least collide because of storage.
    /// We just care about this kind of collision if the account doesn't have code or nonce. Otherwise its value doesn't matter.
    /// For more information see EIP-7610: https://eips.ethereum.org/EIPS/eip-7610
    /// Warning: This attribute should only be used for handling create collisions as it's not necessary appropriate for every scenario. Read the caveat below.
    ///
    /// How this works:
    /// - When getting an account from the DB this is whatever `Database::has_storage` answered
    ///   for it. On an MPT that is "non-empty storage root"; on the EIP-8297 binary trie there is
    ///   no such root and the trie is asked directly. See `LevmAccount::from_account_state`.
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
    pub status: AccountStatus,
    /// Whether this account exists in the state trie.
    /// Used for EIP-7702 auth refund: `account_exists` (EELS) differs from `!is_empty()`.
    /// An account can exist but be empty — the case being an account that holds storage and
    /// nothing else, whose balance, nonce and code are all default. That one is distinguished
    /// from an absent account by `has_storage` alone, which is why the two fields are set
    /// together in `LevmAccount::from_account_state` and why that is a constructor rather than
    /// a `From` impl.
    /// Default is `false` (non-existent); set to `true` when loaded from DB with actual state.
    pub exists: bool,
}

// This is used only in state_v2 runner, storage is already fully filled in the genesis account.
impl From<GenesisAccount> for LevmAccount {
    fn from(genesis: GenesisAccount) -> Self {
        let storage: FxHashMap<H256, U256> = genesis
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
            exists: true,
        }
    }
}
impl LevmAccount {
    /// Build an account from what the database read returned about it.
    ///
    /// # Why `has_storage` is a parameter and not read off `state`
    ///
    /// This used to be `impl From<AccountState>`, deriving the flag as
    /// `state.storage_root != EMPTY_TRIE_HASH`. That derivation is only valid
    /// for an `AccountState` that came out of a merkle-patricia trie, where an
    /// account's storage is its own subtrie and the root of that subtrie is
    /// therefore the boolean. The EIP-8297 binary trie has no per-account
    /// storage root at all, so a read against it reports `EMPTY_TRIE_HASH`
    /// unconditionally and the derivation would answer "no storage" for every
    /// account on the chain. `From` has no database to ask, which is why this
    /// is a constructor: the caller has one. See
    /// `ethrex_vm::VmDatabase::has_storage`.
    ///
    /// # `exists` depends on it too, and less obviously
    ///
    /// Post-EIP-161 a truly empty account is pruned from the trie, so a read
    /// that comes back all-default means "not there". But an account holding
    /// *only* storage — zero balance, zero nonce, no code — is a real account
    /// that must read as existing, and there is exactly one bit distinguishing
    /// it from an absent one. On the MPT that bit used to be its non-empty
    /// `storage_root`, which made `state != default` sufficient. With the field
    /// honest it is `has_storage` and nothing else, so the disjunct below is
    /// load-bearing on the binary path and merely redundant on the MPT (there,
    /// `has_storage` implies a non-empty root implies `state != default`).
    ///
    /// Getting this wrong is silent and consensus-relevant: such an account
    /// would read as non-existent, changing EIP-7702 auth-refund accounting and
    /// letting a `CREATE` land on it. It is also the exact shape EIP-7610
    /// exists to protect, and one a chain can only reach through its genesis
    /// alloc, so no test that merely executes transactions will produce it —
    /// `a_storage_only_account_exists_and_collides_on_both_paths` builds one
    /// deliberately, on both tries.
    pub fn from_account_state(state: AccountState, has_storage: bool) -> Self {
        let is_default = state == AccountState::default();
        LevmAccount {
            info: AccountInfo {
                code_hash: state.code_hash,
                balance: state.balance,
                nonce: state.nonce,
            },
            storage: Default::default(),
            status: AccountStatus::Unmodified,
            has_storage,
            exists: !is_default || has_storage,
        }
    }

    pub fn mark_destroyed(&mut self) {
        self.status = AccountStatus::Destroyed;
    }

    pub fn mark_modified(&mut self) {
        if self.status == AccountStatus::Unmodified {
            self.status = AccountStatus::Modified;
        }
        if self.status == AccountStatus::Destroyed {
            self.status = AccountStatus::DestroyedModified;
        }
        // A modified account exists in the current state
        // (even if it didn't exist in the trie before this tx).
        self.exists = true;
    }

    pub fn has_nonce(&self) -> bool {
        self.info.nonce != 0
    }

    pub fn has_code(&self) -> bool {
        self.info.code_hash != *EMPTY_KECCAK_HASH
    }

    pub fn create_would_collide(&self) -> bool {
        self.has_code() || self.has_nonce() || self.has_storage
    }

    pub fn is_empty(&self) -> bool {
        self.info.is_empty()
    }

    /// Checks if the account is unmodified.
    pub fn is_unmodified(&self) -> bool {
        matches!(self.status, AccountStatus::Unmodified)
    }

    /// Clones the account's metadata (info + flags) but leaves `storage` empty.
    ///
    /// Used on the streaming-executor read-fault path (`load_account`): when the streaming
    /// merkleizer drains `current_accounts_state`, a hot account re-faulted on the next tx would
    /// otherwise deep-copy its entire accumulated storage map (hundreds–thousands of slots) just
    /// so the tx can read the ~3 it touches. Cloning info/flags only and faulting those slots in
    /// lazily avoids that copy. Correctness relies on `get_storage_value` resolving a `current`
    /// miss against `initial_accounts_state` (the committed in-block baseline) before the
    /// pre-block store, which keeps the diff invariant "every key in `current.storage` is also in
    /// `initial.storage`" intact.
    ///
    /// Destructured (not `..`) so adding a field to `LevmAccount` fails to compile here until it
    /// is explicitly carried — a missing flag would silently corrupt the state-transition diff.
    #[inline]
    pub fn clone_without_storage(&self) -> Self {
        let Self {
            info,
            storage: _,
            has_storage,
            status,
            exists,
        } = self;
        Self {
            info: info.clone(),
            storage: FxHashMap::default(),
            has_storage: *has_storage,
            status: status.clone(),
            exists: *exists,
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    #[default]
    /// Account was only read and not mutated at all.
    Unmodified,
    /// Account accessed mutably, doesn't necessarily mean that its state has changed though but it could
    Modified,
    /// Contract executed a SELFDESTRUCT
    Destroyed,
    /// Contract has been destroyed and then modified
    /// This is a particular state because we'll still have in the Database the storage (trie) values but they are actually invalid.
    DestroyedModified,
}
