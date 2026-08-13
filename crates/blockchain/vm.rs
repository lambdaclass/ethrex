use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
    types::{
        AccountState, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code, CodeMetadata,
        pbt_state::BinaryAccount,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_storage::Store;
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
    /// Whether the account holds any storage — the answer
    /// [`VmDatabase::has_storage`] returns, cached alongside the state because
    /// on the binary path the two come out of a single trie open and there is
    /// no way to recover this one from `state` afterwards. See
    /// [`StoreVmDatabase::load_account_state`].
    has_storage: bool,
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
    ///
    /// # Filled, never invalidated — and that is the whole contract
    ///
    /// Every entry is a pure function of ([`Self::state_root`], address), and
    /// `state_root` is written once in [`Self::open`] and never again. So
    /// nothing an execution does can make an entry wrong: this database is a
    /// *snapshot*, not a view of live state, and the state behind that root
    /// does not move while it is open. Invalidating an entry could only cause
    /// the same value to be re-read.
    ///
    /// This is the same shape the spec has. EELS reads through
    /// `TransactionState` -> `BlockState` -> `PreState`, and the `PreState` is
    /// explicitly not modified until `apply_changes_to_state` runs at block
    /// end. What this cache holds is that `PreState`. Mid-block mutations live
    /// one layer up, in `GeneralizedDatabase::current_accounts_state`, which
    /// answers before any read reaches here — the exact counterpart of the
    /// spec's two write layers.
    ///
    /// It is therefore semantically inert: bypassing it entirely (making every
    /// lookup a store read) leaves the whole test suite green. Treat a
    /// suspected staleness bug as a question about the layer above, not this
    /// one. `docs/known_issues.md` has a worked example — a same-block CREATE2
    /// whose answer LEVM's layer changes and this one does not.
    account_state_cache: Arc<RwLock<AccountStateCache>>,
    pub state_root: H256,
    /// Which trie [`Self::state_root`] addresses: the EIP-8297 binary trie when
    /// the header this database was opened at has reached `binaryTreeTime`, the
    /// MPT otherwise.
    ///
    /// **Per header, never per chain.** It is decided once here, from *this*
    /// header's timestamp, and every read below branches on the stored answer. A
    /// header from before the activation genuinely carries an MPT root and has
    /// to keep resolving against the MPT forever — after the flip, across
    /// restarts, and on either side of a reorg. Asking a chain-level question
    /// instead ("is the commitment scheduled", "have we passed it") makes the
    /// whole pre-flip history unreadable, which is why
    /// `pre_flip_headers_keep_executing_against_the_mpt_after_the_flip` exists.
    ///
    /// Nothing maps a block hash to a root here or anywhere else: each header
    /// names the trie that answers for it through `header.state_root` alone.
    binary_tree_active: bool,
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_header: BlockHeader) -> Result<Self, EvmError> {
        Self::open(store, block_header, BTreeMap::new())
    }

    pub fn new_with_block_hash_cache(
        store: Store,
        block_header: BlockHeader,
        block_hash_cache: BTreeMap<BlockNumber, BlockHash>,
    ) -> Result<Self, EvmError> {
        Self::open(store, block_header, block_hash_cache)
    }

    fn open(
        store: Store,
        block_header: BlockHeader,
        block_hash_cache: BTreeMap<BlockNumber, BlockHash>,
    ) -> Result<Self, EvmError> {
        let binary_tree_active = store
            .get_chain_config()
            .is_binary_tree_active(block_header.timestamp);
        let block_hash = block_header.hash();

        // If we don't have the state for the base, we want to fail in a clear way
        // instead of eventually erroring due to one of the several errors that may
        // happen as a result of executing from the wrong state
        // This lets one easily tell apart an inconsistent state from a syncing issue
        //
        // The gate has to ask the same trie the reads will. An active header's
        // `state_root` is a binary root, which names no MPT, so checking the MPT
        // for it would reject every block past the activation before it started.
        let holds_state = if binary_tree_active {
            store.has_binary_trie_state(block_hash, block_header.state_root)
        } else {
            store.has_state_root(block_header.state_root)
        }
        .map_err(|e| EvmError::DB(e.to_string()))?;
        if !holds_state {
            return Err(EvmError::DB(format!(
                "state root missing for block {} (state_root {:#x})",
                block_header.number, block_header.state_root
            )));
        }

        Ok(StoreVmDatabase {
            store,
            block_hash,
            block_hash_cache: Arc::new(Mutex::new(block_hash_cache)),
            account_state_cache: Arc::new(RwLock::new(FxHashMap::default())),
            state_root: block_header.state_root,
            binary_tree_active,
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
            binary_tree_active: false,
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

        let loaded = self.load_account_state(address)?;
        let cached = loaded.map(|(state, has_storage)| AccountStateCacheEntry {
            state,
            hashed_address: H256::from(keccak_hash(address.to_fixed_bytes())),
            has_storage,
        });
        self.account_state_cache
            .write()
            .map_err(|_| EvmError::Custom("LockError".to_string()))?
            .insert(address, cached);
        Ok(cached)
    }

    /// One account, out of whichever trie this database's header addresses,
    /// together with whether it holds any storage.
    ///
    /// The two travel as a pair because on the binary path they are one read
    /// and the second cannot be recovered from the first; see
    /// [`Self::account_state_from_binary_trie`].
    fn load_account_state(
        &self,
        address: Address,
    ) -> Result<Option<(AccountState, bool)>, EvmError> {
        if self.binary_tree_active {
            return Ok(self
                .store
                .get_binary_account(self.state_root, address)
                .map_err(|e| EvmError::DB(e.to_string()))?
                .map(Self::account_state_from_binary_trie));
        }
        // On the MPT the storage root *is* the answer: an account's storage is
        // its own subtrie, so a non-empty root means non-empty storage and
        // nothing extra is read to find that out.
        Ok(self
            .store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| EvmError::DB(e.to_string()))?
            .map(|state| {
                let has_storage = state.storage_root != *EMPTY_TRIE_HASH;
                (state, has_storage)
            }))
    }

    /// Fill an [`AccountState`] from what the binary trie says about an
    /// account, and report the storage question separately.
    ///
    /// # There is no `storage_root` here, so the field reports none
    ///
    /// `AccountState` is MPT-shaped: it summarises an account's storage as the
    /// root of that account's own subtrie. The binary trie has no such value
    /// and cannot grow one — storage there is not a subtrie per account, it is
    /// leaves of the one unified tree, so there is no node whose hash covers
    /// exactly one account's slots.
    ///
    /// So the field gets [`EMPTY_TRIE_HASH`] unconditionally. That is not a
    /// claim that the account has no storage; it is the absence of a claim, and
    /// the only value a field typed as a root can honestly hold when no root
    /// exists. `AccountState` is also what gets RLP-encoded into MPT leaves, so
    /// a value that names no node had no business being in it: a reader that
    /// took it at face value would try to open a trie that is not there. One
    /// did — see `prewarm::warm_merkle_paths`.
    ///
    /// # The boolean goes on its own channel
    ///
    /// What the trie *can* answer is whether the account holds any storage at
    /// all, which is the only thing consumers ever wanted from the field:
    ///
    /// - `LevmAccount::has_storage`, and through it `create_would_collide`,
    ///   which is EIP-7610: a `CREATE` at an address that holds storage but no
    ///   code and a zero nonce must fail. Reaching that needs an account with
    ///   storage and neither code nor nonce, which post-EIP-161 a chain can
    ///   only get from its genesis alloc, since `CREATE` sets the nonce to 1
    ///   before any storage is written.
    /// - `LevmAccount::exists`, which for exactly that account shape is *only*
    ///   true because of its storage — its balance, nonce and code are all
    ///   default.
    /// - the `removed_storage` flag `gen_db` derives from `has_storage` for a
    ///   destroyed-then-modified account.
    ///
    /// All three now read [`VmDatabase::has_storage`]. Pinned by
    /// `a_binary_read_reports_whether_the_account_has_storage` and
    /// `a_storage_only_account_exists_and_collides_on_both_paths`.
    ///
    /// **What it costs.** Two prefix existence checks per account read — one
    /// per storage zone, since an account's slots `0..=63` live in its header
    /// stem and the rest in the storage zone, and no prefix short of the empty
    /// one covers both. Neither scans: `BinaryTrie::contains_prefix` stops at
    /// the first node whose subtree lies wholly under the prefix. The header
    /// check is nearly free, re-walking nodes the account read just loaded; the
    /// overflow one is a genuine extra descent, and only runs when the header
    /// found nothing. Both happen on the account read, not on `has_storage`,
    /// which is served from [`Self::account_state_cache`] — the pairing exists
    /// so that asking the question separately does not cost a second walk.
    fn account_state_from_binary_trie(account: BinaryAccount) -> (AccountState, bool) {
        (
            AccountState {
                nonce: account.info.nonce,
                balance: account.info.balance,
                storage_root: *EMPTY_TRIE_HASH,
                code_hash: account.info.code_hash,
            },
            account.has_storage,
        )
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

    /// Served from the same cache entry the account read fills, so on the
    /// binary path this costs no trie work of its own — the two prefix
    /// existence checks happened when the account was first loaded. On the MPT
    /// path it is `storage_root != EMPTY_TRIE_HASH` and always was; the
    /// difference is that the derivation now lives here rather than at every
    /// call site, which is what lets the binary path answer differently.
    #[instrument(
        level = "trace",
        name = "Account storage presence read",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn has_storage(&self, address: Address) -> Result<bool, EvmError> {
        Ok(self
            .get_cached_account_state_entry(address)?
            .is_some_and(|entry| entry.has_storage))
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

        // The MPT's batch read has a `multi_get` fast path over the flat-KV
        // account table, which is keyed by hashed address and is an MPT
        // structure through and through: the binary trie has no flat mirror to
        // read, and its keys are per-leaf rather than per-account (an account is
        // a basic-data leaf plus a code-hash leaf), so there is no batch shape
        // to hand a backend here. Reading them one at a time is therefore not a
        // fallback but the only form this read has past the activation; each one
        // costs two root-to-leaf walks. A batched read would need the trie to
        // offer one, and belongs with the Phase E node caching rather than here.
        //
        // Both branches yield `(state, has_storage)` pairs so the cache entries
        // this fills are indistinguishable from the ones the single-account
        // path writes — otherwise a `has_storage` call that happened to land on
        // a batch-warmed address would read a flag nobody set.
        let fetched: Vec<Option<(AccountState, bool)>> = if self.binary_tree_active {
            miss_addrs
                .iter()
                .map(|address| self.load_account_state(*address))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            self.store
                .get_account_states_batch_by_root(self.state_root, &miss_addrs)
                .map_err(|e| EvmError::DB(e.to_string()))?
                .into_iter()
                .map(|state| {
                    state.map(|state| {
                        let has_storage = state.storage_root != *EMPTY_TRIE_HASH;
                        (state, has_storage)
                    })
                })
                .collect()
        };

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
            let cached = state.map(|(state, has_storage)| AccountStateCacheEntry {
                state,
                hashed_address: H256::from(keccak_hash(addr.to_fixed_bytes())),
                has_storage,
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
        // The account lookup gates both paths, so an absent account reads as an
        // absent slot whichever trie answers. On the binary path it is not
        // needed to *find* the slot — the leaf's key comes from the address and
        // the slot alone — but keeping it makes the two paths answer alike, and
        // it is served from this database's account cache in all but the first
        // read of an account.
        let Some(entry) = self.get_cached_account_state_entry(address)? else {
            return Ok(None);
        };
        if self.binary_tree_active {
            // No storage root and no per-account subtrie: one leaf, one lookup.
            // `entry.state.storage_root` is deliberately not consulted — it is
            // `EMPTY_TRIE_HASH` for every account here and names no trie to
            // open; see `Self::account_state_from_binary_trie`. In particular
            // it must not be read as "this account has no storage" and
            // short-circuited to `None`: that is what `has_storage` is for.
            return self
                .store
                .get_binary_storage_slot(self.state_root, address, key)
                .map_err(|e| EvmError::DB(e.to_string()));
        }
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
