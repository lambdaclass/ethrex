use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ethrex_binary_trie::{BinaryBackend, BinaryTrieProvider};
use ethrex_common::{
    Address, H256,
    types::{AccountInfo, AccountStateInfo, AccountUpdate, Block, Genesis},
};
use ethrex_crypto::Crypto;
use ethrex_state_backend::{
    AccountMut, BackendKind, CodeReader, MerkleOutput, StateCommitter, StateError, StateReader,
};
use ethrex_trie::{MptBackend, Trie, TrieProvider, genesis_block, genesis_root};

use crate::transition_wiring::TransitionBackend;

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
///
/// Two independent layer caches coexist in `Store` during `Transition` mode —
/// one for the MPT side (frozen) and one for the binary overlay — each keyed by
/// its own root hash. They never cross-read.
pub enum StateBackend {
    Mpt(MptBackend),
    Binary(BinaryBackend),
    /// MPT→binary transition: frozen MPT base + binary overlay.
    /// Reads fall through from overlay to base; writes go to overlay only.
    /// See `transition_wiring.rs` for semantics.
    Transition(Box<TransitionBackend>),
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
        provider: Arc<dyn TrieProvider>,
        code_reader: CodeReader,
    ) -> Self {
        StateBackend::Mpt(MptBackend::new_with_db(
            state_trie,
            crypto,
            provider,
            code_reader,
        ))
    }

    /// Create an in-memory binary trie backend. Used for unit tests and genesis
    /// reconstruction where no DB is available.
    pub fn new_binary() -> Self {
        StateBackend::Binary(BinaryBackend::new())
    }

    /// Create a DB-backed binary trie state backend.
    pub fn new_binary_with_db(
        provider: Arc<dyn BinaryTrieProvider>,
        code_reader: CodeReader,
    ) -> Self {
        StateBackend::Binary(BinaryBackend::new_with_db(provider, code_reader))
    }

    /// Compute the genesis state root for the given backend kind.
    ///
    /// Pure function — does not read stored state. Used by CLI tooling,
    /// deployers, and startup banners that have a [`Genesis`] but no `Store`.
    ///
    /// Returns an error for `BackendKind::Binary` and `BackendKind::Transition`
    /// because binary trie genesis is unsupported (the entry path is only via
    /// the MPT-to-binary transition; there is no fresh-binary start).
    pub fn compute_genesis_root(kind: BackendKind, genesis: &Genesis) -> H256 {
        match kind {
            BackendKind::Mpt => genesis_root(genesis),
            // Binary/Transition genesis is unsupported (entry path is only via transition).
            BackendKind::Binary | BackendKind::Transition => {
                panic!("binary trie genesis is not supported; start in Mpt mode")
            }
        }
    }

    /// Build the genesis [`Block`] for the given backend kind. The embedded
    /// `state_root` matches [`Self::compute_genesis_root`].
    ///
    /// Panics for `BackendKind::Binary | BackendKind::Transition` — see
    /// `compute_genesis_root` for rationale.
    pub fn compute_genesis_block(kind: BackendKind, genesis: &Genesis) -> Block {
        match kind {
            BackendKind::Mpt => genesis_block(genesis),
            // Binary/Transition genesis is unsupported (entry path is only via transition).
            BackendKind::Binary | BackendKind::Transition => {
                panic!("binary trie genesis is not supported; start in Mpt mode")
            }
        }
    }

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
                    }],
                )?;
                continue;
            }

            if update.removed_storage {
                self.clear_storage(update.address)?;
            }

            if let Some(info) = &update.info {
                let mut acct_mut = AccountMut {
                    account: Some(*info),
                    code: None,
                };
                if let Some(code) = &update.code {
                    acct_mut.code = Some(ethrex_state_backend::CodeMut {
                        code: Some(code.bytecode.to_vec()),
                    });
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
    ///
    /// For `Binary` and `Transition` backends, `has_storage` is set conservatively
    /// to `true` when an account exists. The VM reads storage slots individually
    /// regardless of this flag, so a conservative `true` is always correct (it only
    /// skips a minor optimization, never causes an incorrect read path). A precise
    /// derivation would require additional overlay/frozen-MPT plumbing that the VM
    /// does not currently depend on.
    pub fn account_state_info(
        &self,
        addr: Address,
    ) -> Result<Option<AccountStateInfo>, StateError> {
        match self {
            StateBackend::Mpt(b) => Ok(b.account_state(addr)?.map(AccountStateInfo::from)),
            StateBackend::Binary(_) | StateBackend::Transition(_) => {
                Ok(self.account(addr)?.map(|info| AccountStateInfo {
                    info,
                    has_storage: true,
                }))
            }
        }
    }

    // ---- Witness-recording methods ----

    /// Initialize witness recording mode.
    ///
    /// Not supported on `Binary` or `Transition` backends — returns `StateError::Other`.
    pub fn init_witness(&mut self, initial_state_root: H256) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.init_witness(initial_state_root),
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }

    /// Record pre-state accesses for witness generation.
    ///
    /// Not supported on `Binary` or `Transition` backends — returns `StateError::Other`.
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
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }

    /// Apply account updates while recording witness nodes.
    ///
    /// Not supported on `Binary` or `Transition` backends — returns `StateError::Other`.
    pub fn apply_updates_with_witness_state(
        &mut self,
        updates: &[AccountUpdate],
    ) -> Result<MerkleOutput, StateError> {
        match self {
            StateBackend::Mpt(b) => b.apply_witness_updates(updates),
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }

    /// Advance witness recording to the next block's state trie.
    ///
    /// Not supported on `Binary` or `Transition` backends — returns `StateError::Other`.
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
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }

    /// Collect bytecodes from the store for the given code hashes.
    pub fn collect_witness_codes(
        &self,
        store: &crate::Store,
        code_hashes: &[H256],
    ) -> Result<Vec<Vec<u8>>, StateError> {
        match self {
            StateBackend::Mpt(_) => {
                let mut result = Vec::with_capacity(code_hashes.len());
                for &hash in code_hashes {
                    let code = store
                        .get_account_code(hash)
                        .map_err(|e| StateError::Trie(e.to_string()))?
                        .ok_or_else(|| {
                            StateError::Trie(format!("Code not found for hash {hash:?}"))
                        })?;
                    result.push(code.bytecode.to_vec());
                }
                Ok(result)
            }
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }

    /// Finalize and serialize all accumulated witness data into state_proof bytes.
    ///
    /// Consumes `self`. Not supported on `Binary` or `Transition` backends.
    pub fn finalize_witness(
        self,
        touched_accounts: &BTreeMap<Address, Vec<H256>>,
    ) -> Result<Vec<Vec<u8>>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.finalize_witness(touched_accounts),
            StateBackend::Binary(_) | StateBackend::Transition(_) => Err(StateError::Other(
                "witness generation unsupported on binary/transition backend".into(),
            )),
        }
    }
}

impl StateReader for StateBackend {
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.account(addr),
            StateBackend::Binary(b) => b.account(addr),
            StateBackend::Transition(b) => b.account(addr),
        }
    }

    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError> {
        match self {
            StateBackend::Mpt(b) => b.storage(addr, slot),
            StateBackend::Binary(b) => b.storage(addr, slot),
            StateBackend::Transition(b) => b.storage(addr, slot),
        }
    }

    fn code(&self, addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
        match self {
            StateBackend::Mpt(b) => b.code(addr, code_hash),
            StateBackend::Binary(b) => b.code(addr, code_hash),
            StateBackend::Transition(b) => b.code(addr, code_hash),
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
            StateBackend::Binary(b) => b.update_accounts(addrs, muts),
            StateBackend::Transition(b) => b.update_accounts(addrs, muts),
        }
    }

    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.update_storage(addr, slots),
            StateBackend::Binary(b) => b.update_storage(addr, slots),
            StateBackend::Transition(b) => b.update_storage(addr, slots),
        }
    }

    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError> {
        match self {
            StateBackend::Mpt(b) => b.clear_storage(addr),
            StateBackend::Binary(b) => b.clear_storage(addr),
            StateBackend::Transition(b) => b.clear_storage(addr),
        }
    }

    fn hash(&mut self) -> Result<H256, StateError> {
        match self {
            StateBackend::Mpt(b) => b.hash(),
            StateBackend::Binary(b) => b.hash(),
            StateBackend::Transition(b) => b.hash(),
        }
    }

    fn commit(self) -> Result<MerkleOutput, StateError> {
        match self {
            StateBackend::Mpt(b) => b.commit(),
            StateBackend::Binary(b) => b.commit(),
            StateBackend::Transition(b) => b.commit(),
        }
    }
}
