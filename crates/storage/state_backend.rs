use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ethrex_common::{
    Address, H256,
    types::{AccountInfo, AccountStateInfo, AccountUpdate},
};
use ethrex_crypto::Crypto;
use ethrex_state_backend::{AccountMut, MerkleOutput, StateCommitter, StateError, StateReader};
use ethrex_trie::{CodeReader, MptBackend, StorageTrieOpener, Trie};

/// Information collected from the EVM logger needed for witness recording.
pub struct WitnessAccessInfo {
    /// Accounts accessed during execution, mapped to their accessed storage keys.
    pub state_accessed: HashMap<Address, Vec<H256>>,
    /// Bytecode hashes accessed during execution.
    pub code_accessed: Vec<H256>,
    /// Addresses that received withdrawals (must also be recorded in the witness).
    pub withdrawal_addresses: Vec<Address>,
}

/// Backend-agnostic state enum used by `ethrex-storage`.
///
/// Delegates [`StateReader`] and [`StateCommitter`] to the inner backend.
/// New variants (e.g. `Binary`) will be added in future PRs.
pub enum StateBackend {
    Mpt(MptBackend),
}

impl StateBackend {
    /// Create an MPT-backed state backend (in-memory mode).
    pub fn new_mpt(state_trie: Trie, crypto: Arc<dyn Crypto>) -> Self {
        StateBackend::Mpt(MptBackend::new(state_trie, crypto))
    }

    /// Create a DB-backed MPT state backend for on-demand reads.
    pub fn new_mpt_with_db(
        state_trie: Trie,
        crypto: Arc<dyn Crypto>,
        storage_opener: Arc<dyn StorageTrieOpener>,
        code_reader: CodeReader,
    ) -> Self {
        StateBackend::Mpt(MptBackend::new_with_db(
            state_trie,
            crypto,
            storage_opener,
            code_reader,
        ))
    }
    /// Apply account updates and return the new state root + node diffs.
    /// Backend-agnostic: routes through StateCommitter trait methods.
    /// Apply account updates and return the new state root + node diffs.
    /// Backend-agnostic: routes through StateCommitter trait methods.
    pub fn apply_account_updates(
        mut self,
        updates: &[AccountUpdate],
    ) -> Result<MerkleOutput, StateError> {
        for update in updates {
            if update.removed {
                self.update_accounts(
                    &[update.address],
                    &[AccountMut {
                        account: None,
                        code: None,
                        code_size: 0,
                    }],
                )?;
                continue;
            }

            if update.removed_storage {
                self.clear_storage(update.address)?;
            }

            if let Some(info) = &update.info {
                let mut acct_mut = AccountMut {
                    account: Some(info.clone()),
                    code: None,
                    code_size: 0,
                };
                if let Some(code) = &update.code {
                    acct_mut.code = Some(ethrex_state_backend::CodeMut {
                        code: Some(code.bytecode.to_vec()),
                    });
                    acct_mut.code_size = code.bytecode.len();
                }
                self.update_accounts(&[update.address], &[acct_mut])?;
            }

            if !update.added_storage.is_empty() {
                let slots: Vec<(H256, H256)> = update
                    .added_storage
                    .iter()
                    .map(|(k, v)| (*k, H256::from(v.to_big_endian())))
                    .collect();
                self.update_storage(update.address, &slots)?;
            }
        }

        self.commit()
    }

    /// Return backend-agnostic account state info for the VM layer.
    pub fn account_state_info(
        &self,
        addr: Address,
    ) -> Result<Option<AccountStateInfo>, StateError> {
        match self {
            StateBackend::Mpt(b) => Ok(b.account_state(addr)?.map(AccountStateInfo::from)),
        }
    }

    // ---- Witness-recording methods ----

    /// Initialize witness recording mode.
    ///
    /// Wraps the internal state trie with a [`TrieLogger`] and records the
    /// initial state root for use in `finalize_witness`.
    pub fn init_witness(&mut self, initial_state_root: H256) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.init_witness(initial_state_root),
        }
    }

    /// Record pre-state accesses for witness generation.
    ///
    /// For each accessed account, reads it from the logged state trie.
    /// For each account with storage accesses, opens the storage trie at
    /// `parent_hash`, wraps it with a logger, reads the accessed slots,
    /// and stores the logged trie for use in `apply_updates_with_witness_state`.
    /// Also records accounts that received withdrawals.
    pub fn record_witness_accesses(
        &mut self,
        store: &crate::Store,
        parent_hash: H256,
        access_info: &WitnessAccessInfo,
    ) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => {
                // Record withdrawal addresses in the state trie
                for addr in &access_info.withdrawal_addresses {
                    b.record_witness_account(addr)?;
                }

                // Record accessed accounts and their storage
                for (account, acc_keys) in &access_info.state_accessed {
                    b.record_witness_account(account)?;

                    if !acc_keys.is_empty()
                        && let Ok(Some(storage_trie)) = store.storage_trie(parent_hash, *account)
                    {
                        b.record_witness_storage(account, acc_keys, storage_trie)?;
                    }
                }

                Ok(())
            }
        }
    }

    /// Apply account updates while recording witness nodes.
    ///
    /// Uses the internal TrieLogger-wrapped state trie and the accumulated
    /// storage tries. Returns the [`MerkleOutput`] for committing the block.
    ///
    /// After this call, the caller must invoke `advance_witness_to` to
    /// replace the consumed state trie with the next block's trie.
    pub fn apply_updates_with_witness_state(
        &mut self,
        updates: &[AccountUpdate],
    ) -> Result<MerkleOutput, StateError> {
        match self {
            StateBackend::Mpt(b) => b.apply_witness_updates(updates),
        }
    }

    /// Advance witness recording to the next block's state trie.
    ///
    /// Accumulates nodes from the current witness into internal storage,
    /// opens the state trie for `block_hash`, wraps it with a new logger,
    /// and sets it as the active trie for subsequent witness recording.
    pub fn advance_witness_to(
        &mut self,
        store: &crate::Store,
        block_hash: H256,
    ) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => {
                let new_trie = store
                    .state_trie(block_hash)
                    .map_err(|e| StateError::Trie(e.to_string()))?
                    .ok_or_else(|| {
                        StateError::Trie(format!("State trie not found for block {block_hash:?}"))
                    })?;
                b.advance_witness(new_trie)
            }
        }
    }

    /// Collect bytecodes from the store for the given code hashes.
    pub fn collect_witness_codes(
        &self,
        store: &crate::Store,
        code_hashes: &[H256],
    ) -> Result<Vec<Vec<u8>>, StateError> {
        let _ = self; // pattern-match for future backends
        let mut result = Vec::with_capacity(code_hashes.len());
        for &hash in code_hashes {
            let code = store
                .get_account_code(hash)
                .map_err(|e| StateError::Trie(e.to_string()))?
                .ok_or_else(|| StateError::Trie(format!("Code not found for hash {hash:?}")))?;
            result.push(code.bytecode.to_vec());
        }
        Ok(result)
    }

    /// Finalize and serialize all accumulated witness data into state_proof bytes.
    ///
    /// Consumes `self`. Returns the serialized trie nodes as a vector of
    /// RLP-encoded byte vectors.
    pub fn finalize_witness(
        self,
        touched_accounts: &BTreeMap<Address, Vec<H256>>,
    ) -> Result<Vec<Vec<u8>>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.finalize_witness(touched_accounts),
        }
    }
}

impl StateReader for StateBackend {
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.account(addr),
        }
    }

    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError> {
        match self {
            StateBackend::Mpt(b) => b.storage(addr, slot),
        }
    }

    fn code(&self, addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.code(addr, code_hash),
        }
    }
}

impl StateCommitter for StateBackend {
    fn update_accounts(
        &mut self,
        addrs: &[Address],
        muts: &[AccountMut],
    ) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.update_accounts(addrs, muts),
        }
    }

    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.update_storage(addr, slots),
        }
    }

    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.clear_storage(addr),
        }
    }

    fn hash(&mut self) -> Result<H256, StateError> {
        match self {
            StateBackend::Mpt(b) => b.hash(),
        }
    }

    fn commit(self) -> Result<MerkleOutput, StateError> {
        match self {
            StateBackend::Mpt(b) => b.commit(),
        }
    }
}
