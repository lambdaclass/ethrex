use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{AccountState, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code, CodeMetadata},
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_storage::Store;
use ethrex_vm::{EvmError, PrecompileMoves, VmDatabase};
use rustc_hash::FxHashMap;
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tracing::instrument;

/// Per-address overlay applied by [`OverlaidVmDatabase`].
///
/// Each field is independently overridable. `storage_mode` is mutually exclusive
/// per geth semantics: an address picks `Replace` (entire storage replaced; missing
/// slots read zero) or `Diff` (overlay on real storage; missing slots fall through)
/// or `None`.
#[derive(Clone, Debug, Default)]
pub struct StateOverride {
    pub balance: Option<U256>,
    pub nonce: Option<u64>,
    /// Override bytecode together with its precomputed `keccak256(code)` hash.
    pub code: Option<(H256, Code)>,
    pub storage_mode: StorageMode,
    /// `movePrecompileToAddress`: caller of `address` executes the precompile at `target`.
    pub move_precompile_to: Option<Address>,
}

impl StateOverride {
    /// True if this override carries no effective change.
    pub fn is_noop(&self) -> bool {
        self.balance.is_none()
            && self.nonce.is_none()
            && self.code.is_none()
            && matches!(self.storage_mode, StorageMode::None)
            && self.move_precompile_to.is_none()
    }
}

/// Storage override mode. `Replace` short-circuits inner reads; `Diff` overlays.
#[derive(Clone, Debug, Default)]
pub enum StorageMode {
    #[default]
    None,
    /// Replace storage entirely. Slots not in the map read as zero.
    Replace(BTreeMap<H256, U256>),
    /// Overlay on inner storage. Slots not in the map fall through to the inner DB.
    Diff(BTreeMap<H256, U256>),
}

/// `VmDatabase` decorator that applies a set of [`StateOverride`]s on top of an
/// inner database. Used by RPC simulation paths (`eth_call`, `eth_estimateGas`,
/// `eth_createAccessList`, `debug_traceCall`) to honor geth's State Override Set.
///
/// `real_head_number` is the height of the real chain head at construction time.
/// Block-override callers may synthesize a header beyond it; `get_block_hash` returns
/// zero for any block number past `real_head_number` so that `BLOCKHASH` matches geth
/// when the synthetic block sits past the chain tip.
#[derive(Clone)]
pub struct OverlaidVmDatabase<Inner> {
    inner: Inner,
    overrides: Arc<BTreeMap<Address, StateOverride>>,
    real_head_number: BlockNumber,
}

impl<Inner> OverlaidVmDatabase<Inner> {
    pub fn new(
        inner: Inner,
        overrides: BTreeMap<Address, StateOverride>,
        real_head_number: BlockNumber,
    ) -> Self {
        Self {
            inner,
            overrides: Arc::new(overrides),
            real_head_number,
        }
    }

    pub fn inner(&self) -> &Inner {
        &self.inner
    }

    pub fn overrides(&self) -> &BTreeMap<Address, StateOverride> {
        &self.overrides
    }
}

impl<Inner: VmDatabase + Clone> VmDatabase for OverlaidVmDatabase<Inner> {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let base = self.inner.get_account_state(address)?;
        let Some(ov) = self.overrides.get(&address) else {
            return Ok(base);
        };
        if ov.is_noop() {
            return Ok(base);
        }
        // Synthesize an account if the address is unknown on chain but the override
        // gives it state. Other overrides (e.g. movePrecompileToAddress only) keep
        // the account absent.
        let mut state = match base {
            Some(s) => s,
            None if ov.balance.is_some() || ov.nonce.is_some() || ov.code.is_some() => {
                AccountState::default()
            }
            None => return Ok(None),
        };
        if let Some(b) = ov.balance {
            state.balance = b;
        }
        if let Some(n) = ov.nonce {
            state.nonce = n;
        }
        if let Some((h, _)) = &ov.code {
            state.code_hash = *h;
        }
        // storage_root is left untouched: the wrapper intercepts get_storage_slot
        // directly, so the EVM never observes a storage_root that has to match.
        Ok(Some(state))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let Some(ov) = self.overrides.get(&address) else {
            return self.inner.get_storage_slot(address, key);
        };
        match &ov.storage_mode {
            StorageMode::None => self.inner.get_storage_slot(address, key),
            // Replace: missing slots read as zero. Short-circuit; never touch inner.
            StorageMode::Replace(map) => Ok(Some(map.get(&key).copied().unwrap_or_default())),
            // Diff: overlay. Missing slots fall through.
            StorageMode::Diff(map) => match map.get(&key) {
                Some(v) => Ok(Some(*v)),
                None => self.inner.get_storage_slot(address, key),
            },
        }
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        // Geth returns zero for BLOCKHASH(n) where n is past the real chain head.
        if block_number > self.real_head_number {
            return Ok(H256::zero());
        }
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(Code::default());
        }
        // Synthetic-code lookup. The number of overrides per call is small (tens at
        // most), so a linear scan is cheaper than maintaining a parallel map.
        for ov in self.overrides.values() {
            if let Some((h, code)) = &ov.code
                && *h == code_hash
            {
                return Ok(code.clone());
            }
        }
        self.inner.get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        for ov in self.overrides.values() {
            if let Some((h, code)) = &ov.code
                && *h == code_hash
            {
                return Ok(CodeMetadata {
                    length: code.len() as u64,
                });
            }
        }
        self.inner.get_code_metadata(code_hash)
    }
}

/// Derive the precompile-dispatch changes a set of overrides implies, in the form LEVM
/// consumes.
///
/// Both halves come from the same map, matching geth's `StateOverride.Apply`: the
/// `movePrecompileToAddress` relocations, and the suppression of **every** overridden
/// address (geth's `delete(precompiles, addr)`), so overriding a precompile address at all
/// takes it out of the active set.
///
/// Validation of the moves themselves — that the source is a precompile, and that the
/// destination is not itself overridden — happens at the RPC layer, where the active fork
/// is known and the failure can be reported as a bad parameter.
pub fn precompile_moves(overrides: &BTreeMap<Address, StateOverride>) -> PrecompileMoves {
    PrecompileMoves::from_parts(
        overrides
            .iter()
            .filter_map(|(source, ov)| ov.move_precompile_to.map(|dest| (*source, dest))),
        overrides.keys().copied(),
    )
}

/// Helper to compute the synthetic code hash for an override `code` blob.
///
/// Exposed so RPC code can build [`StateOverride::code`] without re-importing
/// the crypto crate.
pub fn synthetic_code(bytecode: Bytes) -> (H256, Code) {
    let hash = H256(keccak_hash(bytecode.as_ref()));
    let code = Code::from_bytecode_unchecked(bytecode, hash);
    (hash, code)
}

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
            return Err(EvmError::DB(format!(
                "state root missing for block {} (state_root {:#x})",
                block_header.number, block_header.state_root
            )));
        }
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(BTreeMap::new())),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
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
            return Err(EvmError::DB(format!(
                "state root missing for block {} (state_root {:#x})",
                block_header.number, block_header.state_root
            )));
        }
        Ok(StoreVmDatabase {
            store,
            block_hash: block_header.hash(),
            block_hash_cache: Arc::new(Mutex::new(block_hash_cache)),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
        })
    }

    /// Build a `StoreVmDatabase` for a given `store` without checking that the
    /// state root exists.  For testing only — the test may not have a real
    /// state but still needs to exercise the code-read path.
    #[cfg(any(test, feature = "testing"))]
    pub fn new_for_test(store: Store) -> Self {
        StoreVmDatabase {
            store,
            block_hash: H256::zero(),
            block_hash_cache: Arc::new(Mutex::new(BTreeMap::new())),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: H256::zero(),
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
            .get_account_state_by_root(self.state_root, address)
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
    fn code_cache_budget_bytes(&self) -> u64 {
        self.store.code_cache_budget_bytes()
    }

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
            .get_storage_at_root_with_known_storage_root(
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
        name = "Account codes batch read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn get_account_codes_batch(&self, code_hashes: &[H256]) -> Result<Vec<Option<Code>>, EvmError> {
        // The empty hash is answered here rather than sent to the store, matching
        // `get_account_code`, so a batch containing EOAs does not fault on it.
        let to_read: Vec<H256> = code_hashes
            .iter()
            .copied()
            .filter(|h| *h != *EMPTY_KECCAK_HASH)
            .collect();
        let read = self
            .store
            .get_account_codes_batch(&to_read)
            .map_err(|e| EvmError::DB(e.to_string()))?;

        let mut by_hash: FxHashMap<H256, Code> = FxHashMap::default();
        for (hash, code) in to_read.iter().zip(read.into_iter()) {
            if let Some(code) = code {
                by_hash.insert(*hash, code);
            }
        }

        Ok(code_hashes
            .iter()
            .map(|h| {
                if *h == *EMPTY_KECCAK_HASH {
                    Some(Code::default())
                } else {
                    by_hash.get(h).cloned()
                }
            })
            .collect())
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

#[cfg(test)]
mod overlaid_db_tests {
    use super::*;
    use std::sync::Mutex;

    /// Minimal in-memory `VmDatabase` for testing the overlay wrapper in isolation.
    #[derive(Clone, Default)]
    struct MockDb {
        accounts: Arc<Mutex<BTreeMap<Address, AccountState>>>,
        storage: Arc<Mutex<BTreeMap<(Address, H256), U256>>>,
        codes: Arc<Mutex<BTreeMap<H256, Code>>>,
        block_hashes: Arc<Mutex<BTreeMap<u64, H256>>>,
    }

    impl VmDatabase for MockDb {
        fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
            Ok(self.accounts.lock().unwrap().get(&address).copied())
        }
        fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
            Ok(self.storage.lock().unwrap().get(&(address, key)).copied())
        }
        fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
            self.block_hashes
                .lock()
                .unwrap()
                .get(&block_number)
                .copied()
                .ok_or_else(|| EvmError::DB(format!("no hash for block {block_number}")))
        }
        fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
            Ok(ChainConfig::default())
        }
        fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
            self.codes
                .lock()
                .unwrap()
                .get(&code_hash)
                .cloned()
                .ok_or_else(|| EvmError::DB(format!("no code for {code_hash:?}")))
        }
        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
            self.codes
                .lock()
                .unwrap()
                .get(&code_hash)
                .map(|c| CodeMetadata {
                    length: c.len() as u64,
                })
                .ok_or_else(|| EvmError::DB(format!("no code for {code_hash:?}")))
        }
    }

    fn addr(byte: u8) -> Address {
        let mut bytes = [0u8; 20];
        bytes[19] = byte;
        Address::from(bytes)
    }

    fn slot(byte: u8) -> H256 {
        let mut bytes = [0u8; 32];
        bytes[31] = byte;
        H256::from(bytes)
    }

    #[test]
    fn balance_override_returns_synthetic_balance() {
        let mock = MockDb::default();
        mock.accounts.lock().unwrap().insert(
            addr(1),
            AccountState {
                balance: U256::from(10),
                ..Default::default()
            },
        );
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(1),
            StateOverride {
                balance: Some(U256::from(999)),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        let state = wrapper.get_account_state(addr(1)).unwrap().unwrap();
        assert_eq!(state.balance, U256::from(999));
    }

    #[test]
    fn nonce_override_returns_synthetic_nonce() {
        let mock = MockDb::default();
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(2),
            StateOverride {
                nonce: Some(42),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        // Address has no real state — wrapper should synthesize.
        let state = wrapper.get_account_state(addr(2)).unwrap().unwrap();
        assert_eq!(state.nonce, 42);
    }

    #[test]
    fn missing_account_with_only_move_precompile_is_still_absent() {
        let mock = MockDb::default();
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(3),
            StateOverride {
                move_precompile_to: Some(addr(0xaa)),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        // movePrecompileToAddress alone doesn't materialize an account.
        assert!(wrapper.get_account_state(addr(3)).unwrap().is_none());
        assert_eq!(
            precompile_moves(wrapper.overrides()).source_for(&addr(0xaa)),
            Some(addr(3))
        );
    }

    /// geth's `StateOverride.Apply` does `delete(precompiles, addr)` for **any** overridden
    /// address, not only the source of a move: overriding a precompile address at all takes
    /// it out of the active precompile set.
    #[test]
    fn overriding_a_precompile_address_at_all_vacates_it() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(4),
            StateOverride {
                balance: Some(U256::one()),
                ..Default::default()
            },
        );
        let moves = precompile_moves(&overrides);
        assert!(
            moves.is_suppressed(&addr(4)),
            "a balance override on a precompile address must vacate it"
        );
        assert_eq!(moves.source_for(&addr(4)), None, "nothing was relocated");
    }

    #[test]
    fn code_override_synthesizes_hash_and_returns_code() {
        let mock = MockDb::default();
        let (hash, code) = synthetic_code(Bytes::from_static(&[0x60, 0x01, 0x60, 0x01, 0x52]));
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(4),
            StateOverride {
                code: Some((hash, code.clone())),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        let state = wrapper.get_account_state(addr(4)).unwrap().unwrap();
        assert_eq!(state.code_hash, hash);
        let fetched = wrapper.get_account_code(hash).unwrap();
        assert_eq!(fetched, code);
        let meta = wrapper.get_code_metadata(hash).unwrap();
        assert_eq!(meta.length as usize, code.len());
    }

    #[test]
    fn replace_mode_short_circuits_missing_slots_to_zero() {
        let mock = MockDb::default();
        // Inner has slot(0) = 0xff at addr(5)
        mock.storage
            .lock()
            .unwrap()
            .insert((addr(5), slot(0)), U256::from(0xff));
        let mut state = BTreeMap::new();
        state.insert(slot(1), U256::from(0xaa));
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(5),
            StateOverride {
                storage_mode: StorageMode::Replace(state),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        // Slot 0 should NOT see the inner 0xff because Replace mode erases it.
        assert_eq!(
            wrapper.get_storage_slot(addr(5), slot(0)).unwrap(),
            Some(U256::zero())
        );
        // Slot 1 sees the override value.
        assert_eq!(
            wrapper.get_storage_slot(addr(5), slot(1)).unwrap(),
            Some(U256::from(0xaa))
        );
    }

    #[test]
    fn diff_mode_overlays_and_falls_through() {
        let mock = MockDb::default();
        mock.storage
            .lock()
            .unwrap()
            .insert((addr(6), slot(0)), U256::from(0xff));
        let mut diff = BTreeMap::new();
        diff.insert(slot(1), U256::from(0xaa));
        let mut overrides = BTreeMap::new();
        overrides.insert(
            addr(6),
            StateOverride {
                storage_mode: StorageMode::Diff(diff),
                ..Default::default()
            },
        );
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        // Diff mode: real slot 0 is preserved.
        assert_eq!(
            wrapper.get_storage_slot(addr(6), slot(0)).unwrap(),
            Some(U256::from(0xff))
        );
        // Diff mode: override slot 1 wins.
        assert_eq!(
            wrapper.get_storage_slot(addr(6), slot(1)).unwrap(),
            Some(U256::from(0xaa))
        );
    }

    #[test]
    fn block_hash_past_real_head_returns_zero() {
        let mock = MockDb::default();
        mock.block_hashes
            .lock()
            .unwrap()
            .insert(50, H256::from_low_u64_be(0xdead));
        let wrapper = OverlaidVmDatabase::new(mock, BTreeMap::new(), 100);
        // <= real head — delegates.
        assert_eq!(
            wrapper.get_block_hash(50).unwrap(),
            H256::from_low_u64_be(0xdead)
        );
        // > real head — zero.
        assert_eq!(wrapper.get_block_hash(150).unwrap(), H256::zero());
    }

    #[test]
    fn noop_override_passes_through_inner_state() {
        let mock = MockDb::default();
        let original = AccountState {
            balance: U256::from(7),
            nonce: 3,
            ..Default::default()
        };
        mock.accounts.lock().unwrap().insert(addr(7), original);
        let mut overrides = BTreeMap::new();
        overrides.insert(addr(7), StateOverride::default());
        let wrapper = OverlaidVmDatabase::new(mock, overrides, 0);
        let state = wrapper.get_account_state(addr(7)).unwrap().unwrap();
        assert_eq!(state.balance, U256::from(7));
        assert_eq!(state.nonce, 3);
    }
}
