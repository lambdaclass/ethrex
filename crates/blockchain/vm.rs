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

/// What a binary-trie read puts in [`AccountState::storage_root`] for an
/// account that holds storage.
///
/// **Not a root.** The binary trie has no per-account storage root to report
/// (see [`StoreVmDatabase::account_state_from_binary_trie`]), and this names no
/// node in any trie. It is the "yes" of a field every consumer reads as
/// `storage_root != EMPTY_TRIE_HASH`, given a value that cannot be mistaken for
/// a hash anything produced: nothing hashes to a run of `0xbb`, so a reader
/// that wrongly opened a trie at it would fail to find one rather than read
/// some other account's.
pub const BINARY_TRIE_HAS_STORAGE: H256 = H256([0xbb; 32]);

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

    /// One account, out of whichever trie this database's header addresses.
    fn load_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        if self.binary_tree_active {
            return Ok(self
                .store
                .get_binary_account(self.state_root, address)
                .map_err(|e| EvmError::DB(e.to_string()))?
                .map(Self::account_state_from_binary_trie));
        }
        self.store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    /// Fill an [`AccountState`] from what the binary trie says about an
    /// account.
    ///
    /// # `storage_root` is a boolean here, not a root
    ///
    /// `AccountState` is MPT-shaped: it summarises an account's storage as the
    /// root of that account's own subtrie. The binary trie has no such value
    /// and cannot grow one — storage there is not a subtrie per account, it is
    /// leaves of the one unified tree, so there is no node whose hash covers
    /// exactly one account's slots.
    ///
    /// What it *can* say is whether the account holds any storage at all, which
    /// is the only thing every consumer of the field asks it — each of them
    /// spells it `storage_root != EMPTY_TRIE_HASH`:
    ///
    /// - `LevmAccount::has_storage`, and through it `create_would_collide`,
    ///   which is EIP-7610: a `CREATE` at an address that holds storage but no
    ///   code and a zero nonce must fail. Reaching that needs an account with
    ///   storage and neither code nor nonce, which post-EIP-161 a chain can
    ///   only get from its genesis alloc, since `CREATE` sets the nonce to 1
    ///   before any storage is written.
    /// - the `removed_storage` flag `gen_db` derives from `has_storage` for a
    ///   destroyed-then-modified account.
    ///
    /// So the field carries the answer to that question and nothing more:
    /// [`EMPTY_TRIE_HASH`] for no storage, and [`BINARY_TRIE_HAS_STORAGE`] —
    /// which is not a root and names no node — for some. Pinned by
    /// `a_binary_read_reports_whether_the_account_has_storage`.
    ///
    /// **Why the marker is safe.** Nothing on the binary path ever opens a trie
    /// at this value: [`VmDatabase::get_storage_slot`] reads the slot's own leaf
    /// from the address and the key, and does not consult the field at all.
    /// A reader that did would fail to find a trie rather than read a wrong
    /// one, since no MPT node hashes to it.
    ///
    /// **What it costs.** Two prefix existence checks per account read — one
    /// per storage zone, since an account's slots `0..=63` live in its header
    /// stem and the rest in the storage zone, and no prefix short of the empty
    /// one covers both. Neither scans: `BinaryTrie::contains_prefix` stops at
    /// the first node whose subtree lies wholly under the prefix. The header
    /// check is nearly free, re-walking nodes the account read just loaded; the
    /// overflow one is a genuine extra descent, and only runs when the header
    /// found nothing.
    fn account_state_from_binary_trie(account: BinaryAccount) -> AccountState {
        AccountState {
            nonce: account.info.nonce,
            balance: account.info.balance,
            storage_root: if account.has_storage {
                BINARY_TRIE_HAS_STORAGE
            } else {
                *EMPTY_TRIE_HASH
            },
            code_hash: account.info.code_hash,
        }
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

        // The MPT's batch read has a `multi_get` fast path over the flat-KV
        // account table, which is keyed by hashed address and is an MPT
        // structure through and through: the binary trie has no flat mirror to
        // read, and its keys are per-leaf rather than per-account (an account is
        // a basic-data leaf plus a code-hash leaf), so there is no batch shape
        // to hand a backend here. Reading them one at a time is therefore not a
        // fallback but the only form this read has past the activation; each one
        // costs two root-to-leaf walks. A batched read would need the trie to
        // offer one, and belongs with the Phase E node caching rather than here.
        let fetched = if self.binary_tree_active {
            miss_addrs
                .iter()
                .map(|address| self.load_account_state(*address))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            self.store
                .get_account_states_batch_by_root(self.state_root, &miss_addrs)
                .map_err(|e| EvmError::DB(e.to_string()))?
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
            // a marker rather than a root here, and opening a trie at it would
            // find nothing; see `Self::account_state_from_binary_trie`.
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
