use crate::EvmError;
use dyn_clone::DynClone;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};

pub trait VmDatabase: Send + Sync + DynClone {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError>;
    fn get_chain_config(&self) -> Result<ChainConfig, EvmError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError>;

    /// Whether `address` holds any storage at all. `false` for an account that
    /// is not there.
    ///
    /// # Why this is not read off `AccountState::storage_root`
    ///
    /// It used to be, spelled `storage_root != EMPTY_TRIE_HASH` at every call
    /// site, and on the MPT that is exactly right: an account's storage is its
    /// own subtrie, so the root of that subtrie *is* the boolean.
    ///
    /// The EIP-8297 binary trie has no such value and cannot grow one — storage
    /// there is not a subtrie per account but leaves of the one unified tree,
    /// so no node's hash covers exactly one account's slots. A binary-path read
    /// therefore reports [`EMPTY_TRIE_HASH`] for *every* account, storage or
    /// not, because that is the only thing it can honestly put in a field
    /// typed as a root. The boolean has to travel separately or not at all.
    ///
    /// [`EMPTY_TRIE_HASH`]: ethrex_common::constants::EMPTY_TRIE_HASH
    ///
    /// # Why it has no default implementation
    ///
    /// A default deriving the answer from `storage_root` would be right for
    /// every MPT-backed implementation and silently wrong for a binary-backed
    /// one — reporting "no storage" for the whole chain past the activation,
    /// which turns off EIP-7610's create-collision check and the
    /// destroyed-account storage wipe. That failure is invisible until a
    /// `CREATE` lands on one of the handful of storage-only accounts a genesis
    /// alloc can produce. This trait is the exact seam where an MPT-shaped
    /// struct meets a trie that is not an MPT, so the question is asked of
    /// every implementation explicitly.
    ///
    /// An MPT-only implementation answers it in one line:
    /// `Ok(self.get_account_state(address)?.is_some_and(|s| s.storage_root != *EMPTY_TRIE_HASH))`.
    fn has_storage(&self, address: Address) -> Result<bool, EvmError>;

    /// Batch account-state lookup. Default impl loops `get_account_state`.
    /// Backends that can amortize per-key cost (e.g. rocksdb `multi_get_cf` on
    /// the flat key-value table) should override this.
    fn get_account_states_batch(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Option<AccountState>>, EvmError> {
        addresses
            .iter()
            .map(|a| self.get_account_state(*a))
            .collect()
    }

    /// Batch storage-slot lookup. Default impl loops `get_storage_slot`.
    /// Backends that can amortize per-key cost (e.g. rocksdb `multi_get_cf` on
    /// the flat key-value table) should override this.
    fn get_storage_slots_batch(
        &self,
        keys: &[(Address, H256)],
    ) -> Result<Vec<Option<U256>>, EvmError> {
        keys.iter()
            .map(|&(addr, key)| self.get_storage_slot(addr, key))
            .collect()
    }
}

dyn_clone::clone_trait_object!(VmDatabase);

pub type DynVmDatabase = Box<dyn VmDatabase + Send + Sync + 'static>;
