use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{
        AccountStateInfo, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code, CodeMetadata,
    },
};
use ethrex_state_backend::{BackendKind, StateReader};
use ethrex_storage::{StateBackend, Store};
use ethrex_vm::{EvmError, VmDatabase};
use rustc_hash::FxHashMap;
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tracing::instrument;

type AccountStateCache = FxHashMap<Address, Option<AccountStateInfo>>;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    // Used to store known block hashes during execution as we look them up when executing BLOCKHASH opcode
    // We will also pre-load this when executing blocks in batches, as we will only add the blocks at the end
    // and may need to access hashes of blocks previously executed in the batch
    pub block_hash_cache: Arc<Mutex<BTreeMap<BlockNumber, BlockHash>>>,
    /// Memoized account state info for storage reads.
    /// Avoids repeated state-trie account decodes when reading many slots
    /// from the same account during execution.
    account_state_cache: Arc<RwLock<AccountStateCache>>,
    pub state_root: H256,
    /// DB-backed state reader for the pre-block state, used for account and
    /// storage reads during execution.
    state_backend: Arc<StateBackend>,
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_header: BlockHeader) -> Result<Self, EvmError> {
        let state_backend = Self::build_state_backend(&store, &block_header)?;
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(BTreeMap::new())),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
            state_backend: Arc::new(state_backend),
        })
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_header: BlockHeader,
        block_hash_cache: BTreeMap<BlockNumber, BlockHash>,
    ) -> Result<Self, EvmError> {
        let state_backend = Self::build_state_backend(&store, &block_header)?;
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(block_hash_cache)),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
            state_backend: Arc::new(state_backend),
        })
    }

    /// Constructs the correct `StateBackend` for the given block header, dispatching
    /// on the store's current backend kind.
    ///
    /// For `BackendKind::Mpt`: performs the `has_state_root` pre-check (fast-fail on
    /// missing prestate) then opens an MPT reader for the block's `state_root`.
    ///
    /// For `BackendKind::Transition`: skips the `has_state_root` check (the header's
    /// `state_root` is the canonical MPT root from the peer, which is never written to
    /// disk post-switch — the check would always fail). Opens a transition reader using
    /// the metadata persisted at activation time.
    ///
    /// `BackendKind::Binary` is not reachable from the VM read path until Phase 8
    /// (genesis-binary) lands.
    fn build_state_backend(
        store: &Store,
        block_header: &BlockHeader,
    ) -> Result<StateBackend, EvmError> {
        match store.backend_kind() {
            BackendKind::Mpt => {
                // If we don't have the state for the base, we want to fail in a clear way
                // instead of eventually erroring due to one of the several errors that may
                // happen as a result of executing from the wrong state.
                // This lets one easily tell apart an inconsistent state from a syncing issue.
                if !store
                    .has_state_root(block_header.state_root)
                    .map_err(|e| EvmError::DB(e.to_string()))?
                {
                    return Err(EvmError::DB("state root missing".to_string()));
                }
                store
                    .new_state_reader(block_header.state_root)
                    .map_err(|e| EvmError::DB(e.to_string()))
            }
            BackendKind::Transition => {
                let (switch_block, frozen_mpt_root, binary_root) =
                    store.transition_metadata().ok_or_else(|| {
                        EvmError::DB(
                            "Transition mode requires transition_metadata; not loaded".to_string(),
                        )
                    })?;
                store
                    .new_transition_state_reader(switch_block, frozen_mpt_root, binary_root)
                    .map_err(|e| EvmError::DB(e.to_string()))
            }
            BackendKind::Binary => unreachable!(
                "BackendKind::Binary unreachable from VM read path until Phase 8 (genesis-binary) lands"
            ),
        }
    }

    fn get_cached_account_state_info(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateInfo>, EvmError> {
        if let Some(entry) = self
            .account_state_cache
            .read()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?
            .get(&address)
            .copied()
        {
            return Ok(entry);
        }

        let loaded = self
            .state_backend
            .account_state_info(address)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        self.account_state_cache
            .write()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?
            .insert(address, loaded);
        Ok(loaded)
    }
}

impl VmDatabase for StoreVmDatabase {
    #[instrument(
        level = "trace",
        name = "Account read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_account_state(&self, address: Address) -> Result<Option<AccountStateInfo>, EvmError> {
        self.get_cached_account_state_info(address)
    }

    #[instrument(
        level = "trace",
        name = "Storage read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let Some(_) = self.get_cached_account_state_info(address)? else {
            return Ok(None);
        };
        let value = self
            .state_backend
            .storage(address, key)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        if value.is_zero() {
            Ok(None)
        } else {
            Ok(Some(U256::from_big_endian(value.as_bytes())))
        }
    }

    #[instrument(
        level = "trace",
        name = "Block hash read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        let mut block_hash_cache = self
            .block_hash_cache
            .lock()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?;
        // Check if we have it cached
        if let Some(block_hash) = block_hash_cache.get(&block_number) {
            return Ok(*block_hash);
        }
        // First check if our block is canonical, if it is then it's ancestor will also be canonical and we can look it up directly
        if self
            .store
            .is_canonical_sync(self.block_hash)
            .map_err(|err| EvmError::DB(err.to_string()))?
        {
            if let Some(hash) = self
                .store
                .get_canonical_block_hash_sync(block_number)
                .map_err(|err| EvmError::DB(err.to_string()))?
            {
                block_hash_cache.insert(block_number, hash);
                return Ok(hash);
            }
        // If our block is not canonical then we must look for the target in our block's ancestors
        } else {
            // Find the oldest known hash after the target block to shortcut the lookup
            let oldest_succesor = block_hash_cache
                .iter()
                .find_map(|(key, hash)| (*key > block_number).then_some(*hash))
                .unwrap_or(self.block_hash);
            for ancestor_res in self.store.ancestors(oldest_succesor) {
                let (hash, ancestor) = ancestor_res.map_err(|e| EvmError::DB(e.to_string()))?;
                block_hash_cache.insert(ancestor.number, hash);
                match ancestor.number.cmp(&block_number) {
                    Ordering::Greater => continue,
                    Ordering::Equal => return Ok(hash),
                    Ordering::Less => {
                        return Err(EvmError::DB(format!(
                            "Block number requested {block_number} is higher than the current block number {}",
                            ancestor.number
                        )));
                    }
                }
            }
        }
        // Block not found
        Err(EvmError::DB(format!(
            "Block hash not found for block number {block_number}"
        )))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.store.get_chain_config())
    }

    #[instrument(
        level = "trace",
        name = "Account code read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        match self.store.get_account_code(code_hash) {
            Ok(Some(code)) => Ok(code),
            Ok(None) => Err(EvmError::DB(format!(
                "Code not found for hash: {code_hash:?}",
            ))),
            Err(e) => Err(EvmError::DB(e.to_string())),
        }
    }

    #[instrument(
        level = "trace",
        name = "Code metadata read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        use ethrex_common::constants::EMPTY_KECCACK_HASH;

        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        match self.store.get_code_metadata(code_hash) {
            Ok(Some(metadata)) => Ok(metadata),
            Ok(None) => Err(EvmError::DB(format!(
                "Code metadata not found for hash: {code_hash:?}",
            ))),
            Err(e) => Err(EvmError::DB(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use ethrex_common::{Address, H256, types::BlockHeader};
    use ethrex_state_backend::BackendKind;
    use ethrex_storage::{EngineType, Store};
    use ethrex_trie::EMPTY_TRIE_HASH;
    use ethrex_vm::VmDatabase;

    use super::StoreVmDatabase;

    /// Verifies that `StoreVmDatabase::new` in Transition mode bypasses the
    /// `has_state_root` gate and that `get_account_state` succeeds through the
    /// Transition read path (sub-Bug 0B: `account_state_info` previously returned
    /// an error for Binary/Transition backends, making every account read fail).
    ///
    /// Bug 0 root cause: both constructors called `store.new_state_reader` unconditionally,
    /// so Transition mode was cosmetic — block execution still ran the MPT reader and the
    /// `has_state_root` check always failed post-switch (the peer header's MPT root is
    /// never stored as a state trie root after activation).
    ///
    /// This test proves:
    /// 1. In Transition mode, `StoreVmDatabase::new` succeeds even when the header's
    ///    `state_root` is not present in the DB (gate correctly bypassed).
    /// 2. `get_account_state` returns `Ok(None)` for an unknown address rather than
    ///    erroring with "AccountStateInfo is MPT-specific" (sub-Bug 0B fixed).
    /// 3. In MPT mode with the same non-canonical `state_root`, `StoreVmDatabase::new`
    ///    fails with "state root missing" (gate fires as expected).
    #[test]
    fn transition_mode_vm_database_uses_transition_reader() {
        // Build an MPT store and activate transition by persisting metadata +
        // hot-swapping backend_kind.
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to create in-memory store");

        // Use EMPTY_TRIE_HASH as the frozen MPT root so that the TransitionBackend's
        // base MPT lookups return Ok(None) for any unknown address (the trie exists as
        // an empty trie; no on-disk node data required). This lets get_account_state
        // exercise the full read path without needing actual MPT state written to disk.
        let frozen_mpt_root = *EMPTY_TRIE_HASH;
        let binary_root = H256::zero(); // fresh binary overlay (no commits yet)

        store
            .persist_transition_metadata(100, frozen_mpt_root, binary_root)
            .expect("persist_transition_metadata failed");
        store.set_backend_kind(BackendKind::Transition);

        // The header's state_root is set to a non-canonical value that does NOT match
        // any actual trie root on disk. In Transition mode this field is ignored for
        // backend construction (the metadata's frozen_mpt_root is used instead); in
        // MPT mode the `has_state_root` gate would reject it.
        let non_canonical_root = H256::from([0xAA; 32]);
        let header = {
            let mut h = BlockHeader::default();
            h.state_root = non_canonical_root;
            h
        };

        // --- Positive assertion: Transition mode must succeed. ---
        // Gate is bypassed; TransitionBackend is built from persisted metadata.
        let db = StoreVmDatabase::new(store.clone(), header.clone())
            .expect("StoreVmDatabase::new must succeed in Transition mode");

        // --- Sub-Bug 0B: account_state_info must not error for Transition backend. ---
        // Prior to the fix, this call returned Err("AccountStateInfo is MPT-specific…")
        // for any address, causing every block-execution account read to fail post-switch.
        // With frozen_mpt_root = EMPTY_TRIE_HASH, the overlay is empty and the base MPT
        // lookup returns Ok(None) without any DB I/O. The overall result must be Ok(None).
        let unknown_addr = Address::from([0xBB; 20]);
        let account_result = db.get_account_state(unknown_addr);
        assert!(
            account_result.is_ok(),
            "get_account_state must not error on Transition backend; got: {account_result:?}"
        );
        assert_eq!(
            account_result.unwrap(),
            None,
            "get_account_state must return None for an unknown address in empty Transition state"
        );

        // --- Negative assertion: MPT mode with the same non-canonical root must fail. ---
        // This proves the gate actually fires in MPT mode, making the Transition bypass
        // meaningful rather than vacuous.
        let mpt_store = Store::new(".", EngineType::InMemory, BackendKind::Mpt)
            .expect("failed to create MPT store");
        let mpt_result = StoreVmDatabase::new(mpt_store, header);
        assert!(
            mpt_result.is_err(),
            "StoreVmDatabase::new must fail in MPT mode for a non-canonical state_root"
        );
        if let Err(ethrex_vm::EvmError::DB(msg)) = mpt_result {
            assert!(
                msg.contains("state root missing"),
                "error must say 'state root missing'; got: {msg}"
            );
        } else {
            panic!("expected EvmError::DB(state root missing)");
        }
    }
}
