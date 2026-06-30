use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{AccountState, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_storage::{StorageReadSession, Store};
use ethrex_vm::{EvmError, VmDatabase};
use rustc_hash::FxHashMap;
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tracing::instrument;

#[derive(Clone, Copy)]
struct AccountStateCacheEntry {
    state: AccountState,
    hashed_address: H256,
}

type AccountStateCache = FxHashMap<Address, Option<AccountStateCacheEntry>>;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    // Used to store known block hashes during execution as we look them up when executing BLOCKHASH opcode
    // We will also pre-load this when executing blocks in batches, as we will only add the blocks at the end
    // and may need to access hashes of blocks previously executed in the batch
    pub block_hash_cache: Arc<Mutex<BTreeMap<BlockNumber, BlockHash>>>,
    /// Memoized account states and hashed addresses for storage reads.
    /// This avoids repeated state-trie account decodes when reading many slots
    /// from the same account during execution.
    account_state_cache: Arc<RwLock<AccountStateCache>>,
    pub state_root: H256,
    /// Snapshot of read resources at `state_root`, acquired once at construction
    /// so per-opcode account/storage reads don't re-lock or re-open the backend.
    read_session: StorageReadSession,
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_header: BlockHeader) -> Result<Self, EvmError> {
        // If we don't have the state for the base, we want to fail in a clear way
        // instead of eventually erroring due to one of the several errors that may
        // happen as a result of executing from the wrong state
        // This lets one easily tell apart an inconsistent state from a syncing issue
        if !store
            .has_state_root(block_header.state_root)
            .map_err(|e| EvmError::DB(e.to_string()))?
        {
            return Err(EvmError::DB("state root missing".to_string()));
        }
        let read_session = store
            .begin_storage_read_session(block_header.state_root)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(BTreeMap::new())),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
            read_session,
        })
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_header: BlockHeader,
        block_hash_cache: BTreeMap<BlockNumber, BlockHash>,
    ) -> Result<Self, EvmError> {
        // Fail clearly if prestate is missing. See `StoreVmDatabase::new` for details on why we want this
        if !store
            .has_state_root(block_header.state_root)
            .map_err(|e| EvmError::DB(e.to_string()))?
        {
            return Err(EvmError::DB("state root missing".to_string()));
        }
        let read_session = store
            .begin_storage_read_session(block_header.state_root)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(block_hash_cache)),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
            read_session,
        })
    }

    /// Build a `StoreVmDatabase` for a given `store` without checking that the
    /// state root exists.  For testing only — the test may not have a real
    /// state but still needs to exercise the code-read path.
    #[cfg(any(test, feature = "testing"))]
    pub fn new_for_test(store: Store) -> Self {
        let read_session = store
            .begin_storage_read_session(H256::zero())
            .expect("failed to open test read session");
        StoreVmDatabase {
            store,
            block_hash: H256::zero(),
            block_hash_cache: Arc::new(Mutex::new(BTreeMap::new())),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: H256::zero(),
            read_session,
        }
    }

    fn get_cached_account_state_entry(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateCacheEntry>, EvmError> {
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
            .store
            .get_account_state_with_session(&self.read_session, self.state_root, address)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        let cached = loaded.map(|state| AccountStateCacheEntry {
            state,
            hashed_address: H256::from(keccak_hash(address.to_fixed_bytes())),
        });
        self.account_state_cache
            .write()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?
            .insert(address, cached);
        Ok(cached)
    }
}

impl VmDatabase for StoreVmDatabase {
    #[instrument(
        level = "trace",
        name = "Account read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        Ok(self
            .get_cached_account_state_entry(address)?
            .map(|entry| entry.state))
    }

    #[instrument(
        level = "trace",
        name = "Account read batch",
        skip_all,
        fields(namespace = "block_execution", n = addresses.len())
    )]
    fn get_account_states_batch(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Option<AccountState>>, EvmError> {
        // Split into cached / uncached so the rocksdb multi_get only fires for
        // addresses we haven't memoized yet on this StoreVmDatabase.
        let mut results: Vec<Option<AccountState>> = vec![None; addresses.len()];
        let mut miss_idx: Vec<usize> = Vec::new();
        let mut miss_addrs: Vec<Address> = Vec::new();
        {
            let cache = self
                .account_state_cache
                .read()
                .map_err(|_| EvmError::Custom("LockError".to_string()))?;
            for (i, addr) in addresses.iter().enumerate() {
                match cache.get(addr) {
                    Some(Some(entry)) => results[i] = Some(entry.state),
                    Some(None) => results[i] = None,
                    None => {
                        miss_idx.push(i);
                        miss_addrs.push(*addr);
                    }
                }
            }
        }

        if miss_addrs.is_empty() {
            return Ok(results);
        }

        let fetched = self
            .store
            .get_account_states_batch_by_root(self.state_root, &miss_addrs)
            .map_err(|e| EvmError::DB(e.to_string()))?;

        // Populate the per-DB cache and assemble results. `insert` (vs `or_insert`)
        // is intentional: `state_root` is fixed for this `StoreVmDatabase`, so a
        // concurrent populator can only have written the same value for the same
        // address — overwriting is a no-op, and the unconditional insert avoids
        // the extra `entry`-API lookup.
        let mut cache = self
            .account_state_cache
            .write()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?;
        for ((slot, addr), state) in miss_idx
            .iter()
            .zip(miss_addrs.iter())
            .zip(fetched.into_iter())
        {
            let cached = state.map(|state| AccountStateCacheEntry {
                state,
                hashed_address: H256::from(keccak_hash(addr.to_fixed_bytes())),
            });
            cache.insert(*addr, cached);
            results[*slot] = cached.map(|e| e.state);
        }

        Ok(results)
    }

    #[instrument(
        level = "trace",
        name = "Storage read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let Some(entry) = self.get_cached_account_state_entry(address)? else {
            return Ok(None);
        };
        self.store
            .get_storage_with_session(
                &self.read_session,
                self.state_root,
                entry.hashed_address,
                entry.state.storage_root,
                key,
            )
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    #[instrument(
        level = "trace",
        name = "Storage read batch",
        skip_all,
        fields(namespace = "block_execution", n = keys.len())
    )]
    fn get_storage_slots_batch(
        &self,
        keys: &[(Address, H256)],
    ) -> Result<Vec<Option<U256>>, EvmError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve the account state (hashed address + storage root) for each
        // distinct address. This mirrors the per-slot `get_storage_slot` path,
        // which opens the storage trie from the cached account entry. Slots for
        // a non-existent account resolve to `None`, exactly as the single-get
        // path returns `None` when the account entry is missing.
        let mut entries: FxHashMap<Address, Option<AccountStateCacheEntry>> = FxHashMap::default();
        for &(addr, _) in keys {
            if let std::collections::hash_map::Entry::Vacant(slot) = entries.entry(addr) {
                slot.insert(self.get_cached_account_state_entry(addr)?);
            }
        }

        // Build the store-batch input for slots whose account exists, remembering
        // the original index so results can be scattered back in input order.
        let mut results: Vec<Option<U256>> = vec![None; keys.len()];
        let mut batch_idx: Vec<usize> = Vec::with_capacity(keys.len());
        let mut batch: Vec<(H256, H256, H256)> = Vec::with_capacity(keys.len());
        for (i, &(addr, key)) in keys.iter().enumerate() {
            if let Some(Some(entry)) = entries.get(&addr) {
                batch_idx.push(i);
                batch.push((entry.hashed_address, entry.state.storage_root, key));
            }
        }

        if batch.is_empty() {
            return Ok(results);
        }

        let fetched = self
            .store
            .get_storage_values_batch_by_root(self.state_root, &batch)
            .map_err(|e| EvmError::DB(e.to_string()))?;
        for (i, value) in batch_idx.into_iter().zip(fetched.into_iter()) {
            results[i] = value;
        }

        Ok(results)
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
        if code_hash == *EMPTY_KECCAK_HASH {
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
        use ethrex_common::constants::EMPTY_KECCAK_HASH;

        if code_hash == *EMPTY_KECCAK_HASH {
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
