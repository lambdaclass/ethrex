use std::collections::BTreeMap;
use std::sync::Mutex;

use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(feature = "rocksdb")]
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountInfo, AccountState, AccountUpdate, Code, GenesisAccount},
    utils::keccak,
};

#[cfg(feature = "rocksdb")]
use crate::node_store::NodeStore;
use crate::{
    BinaryTrie,
    error::BinaryTrieError,
    key_mapping::{
        chunkify_code, get_tree_key_for_basic_data, get_tree_key_for_code_chunk,
        get_tree_key_for_code_hash, get_tree_key_for_storage_slot, pack_basic_data,
        unpack_basic_data,
    },
    merkle::merkelize,
    node::NodeId,
};

// ── Diff layer types ──────────────────────────────────────────────────

/// Result of looking up a value in the diff tree.
pub enum DiffLookup<T> {
    /// Value found in a diff layer.
    Found(T),
    /// Explicitly deleted in a diff layer.
    Deleted,
    /// Walked all the way to the base without finding the key.
    /// Safe to read from the base trie.
    NotModified,
    /// Block hash not found in the in-memory diff tree.
    /// Caller must try loading from disk or return an error.
    NotInMemory,
}

/// State changes from a single block.
#[derive(Default)]
pub struct StateDiff {
    /// Accounts modified: address -> post-state. None = account deleted.
    pub accounts: FxHashMap<Address, Option<AccountState>>,
    /// Storage slots modified: (address, key) -> post-value. None = zeroed.
    pub storage: FxHashMap<(Address, H256), Option<U256>>,
    /// Code deployed: code_hash -> bytecode.
    pub code: FxHashMap<H256, Bytes>,
    /// Addresses whose storage was fully cleared (SELFDESTRUCT).
    /// Acts as a blanket tombstone: any storage slot not explicitly re-set
    /// in this layer's `storage` map is treated as deleted. Individual
    /// per-slot tombstones are NOT needed when this flag is set.
    pub storage_cleared: FxHashSet<Address>,
}

/// A node in the diff layer tree.
struct DiffTreeNode {
    parent_hash: H256,
    block_number: u64,
    diff: StateDiff,
}

/// Tree of per-block state diffs for recent blocks.
///
/// Supports branching: multiple blocks can share the same parent.
/// Reads walk from the target block backwards through parent links
/// until the value is found or we reach the base.
pub struct DiffTree {
    layers: FxHashMap<H256, DiffTreeNode>,
    /// Block hash of the base. State at the base is in the flushed trie on disk.
    pub base_hash: H256,
    /// Block number of the base.
    pub base_block: u64,
}

impl DiffTree {
    pub fn new() -> Self {
        Self {
            layers: FxHashMap::default(),
            base_hash: H256::zero(),
            base_block: 0,
        }
    }

    /// Look up an account at a specific block.
    pub fn get_account(&self, addr: &Address, at: H256) -> DiffLookup<AccountState> {
        let mut current = at;
        loop {
            if current == self.base_hash {
                return DiffLookup::NotModified;
            }
            match self.layers.get(&current) {
                Some(node) => match node.diff.accounts.get(addr) {
                    Some(Some(state)) => return DiffLookup::Found(state.clone()),
                    Some(None) => return DiffLookup::Deleted,
                    None => current = node.parent_hash,
                },
                None => return DiffLookup::NotInMemory,
            }
        }
    }

    /// Look up a storage slot at a specific block.
    pub fn get_storage(&self, addr: &Address, key: H256, at: H256) -> DiffLookup<U256> {
        let mut current = at;
        loop {
            if current == self.base_hash {
                return DiffLookup::NotModified;
            }
            match self.layers.get(&current) {
                Some(node) => {
                    // Check explicit storage entry first.
                    match node.diff.storage.get(&(*addr, key)) {
                        Some(Some(val)) => return DiffLookup::Found(*val),
                        Some(None) => return DiffLookup::Deleted,
                        None => {
                            // If this layer cleared all storage for the address,
                            // any slot not explicitly re-set is deleted.
                            if node.diff.storage_cleared.contains(addr) {
                                return DiffLookup::Deleted;
                            }
                            current = node.parent_hash;
                        }
                    }
                }
                None => return DiffLookup::NotInMemory,
            }
        }
    }

    /// Look up code at a specific block.
    pub fn get_code(&self, code_hash: &H256, at: H256) -> DiffLookup<Bytes> {
        let mut current = at;
        loop {
            if current == self.base_hash {
                return DiffLookup::NotModified;
            }
            match self.layers.get(&current) {
                Some(node) => match node.diff.code.get(code_hash) {
                    Some(code) => return DiffLookup::Found(code.clone()),
                    None => current = node.parent_hash,
                },
                None => return DiffLookup::NotInMemory,
            }
        }
    }

    /// Insert a new diff layer.
    pub fn add_layer(
        &mut self,
        block_hash: H256,
        parent_hash: H256,
        block_number: u64,
        diff: StateDiff,
    ) {
        self.layers.insert(
            block_hash,
            DiffTreeNode {
                parent_hash,
                block_number,
                diff,
            },
        );
    }

    /// Remove layers older than `cutoff_block`.
    pub fn prune_before(&mut self, cutoff_block: u64) {
        self.layers
            .retain(|_, node| node.block_number > cutoff_block);
    }
}

// Key prefixes for code_store and storage_keys in RocksDB.
// 0x01 is used by NodeStore for trie nodes; 0xFF for trie metadata.
#[cfg(feature = "rocksdb")]
const CODE_PREFIX: u8 = 0x02;
#[cfg(feature = "rocksdb")]
const STORAGE_KEYS_PREFIX: u8 = 0x03;
#[cfg(feature = "rocksdb")]
const META_BLOCK_KEY: &[u8] = &[0xFF, b'B'];
#[cfg(feature = "rocksdb")]
const DIFF_PREFIX: u8 = 0x04;
#[cfg(feature = "rocksdb")]
const META_BASE_HASH_KEY: &[u8] = &[0xFF, b'H'];

impl std::fmt::Debug for BinaryTrieState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryTrieState").finish_non_exhaustive()
    }
}

pub struct BinaryTrieState {
    /// The underlying binary trie holding all state leaves.
    trie: BinaryTrie,

    /// Tree of per-block state diffs for recent blocks.
    diff_tree: DiffTree,

    /// Trie root at the last flush. Used for base reads when a value
    /// isn't found in any diff layer.
    base_root: Option<NodeId>,

    /// Code by keccak256 hash — for fast `get_account_code` lookups.
    /// Code is also chunked in the trie, but reconstructing from chunks
    /// on every CALL would be expensive.
    ///
    /// INVARIANT: every non-empty code_hash leaf in the trie must have a
    /// corresponding entry here. This is maintained by `apply_genesis` and
    /// `apply_account_update`. Any future deserialization or state-loading
    /// path must also populate this map, or `get_account_code` will fail.
    ///
    /// Wrapped in a Mutex so `get_account_code` can populate the cache with
    /// `&self` (concurrent reads from executor threads).
    code_store: Mutex<FxHashMap<H256, Bytes>>,

    /// Tracks which storage keys each account has written.
    /// Needed for `removed_storage` (SELFDESTRUCT) since the binary trie
    /// has no prefix-enumeration — we can't discover all storage keys
    /// for an address without this side structure.
    ///
    /// Wrapped in a Mutex so `has_storage_keys` can populate the cache with
    /// `&self` (concurrent reads from executor threads).
    storage_keys: Mutex<FxHashMap<Address, FxHashSet<H256>>>,

    /// Shared RocksDB handle (present only when opened with `open()`).
    #[cfg(feature = "rocksdb")]
    db: Option<Arc<rocksdb::DB>>,

    /// Code hashes written since the last `flush()`.
    #[cfg(feature = "rocksdb")]
    dirty_codes: FxHashSet<H256>,

    /// Addresses whose storage_keys entry has changed since the last `flush()`.
    #[cfg(feature = "rocksdb")]
    dirty_storage_keys: FxHashSet<Address>,

    /// Number of blocks applied since the last flush.
    #[cfg(feature = "rocksdb")]
    blocks_since_flush: u64,

    /// Flush to disk when `blocks_since_flush` reaches this threshold.
    /// Default 128, matching MPT's `DB_COMMIT_THRESHOLD`.
    #[cfg(feature = "rocksdb")]
    flush_threshold: u64,
}

impl BinaryTrieState {
    pub fn new() -> Self {
        Self {
            trie: BinaryTrie::new(),
            diff_tree: DiffTree::new(),
            base_root: None,
            code_store: Mutex::new(FxHashMap::default()),
            storage_keys: Mutex::new(FxHashMap::default()),
            #[cfg(feature = "rocksdb")]
            db: None,
            #[cfg(feature = "rocksdb")]
            dirty_codes: FxHashSet::default(),
            #[cfg(feature = "rocksdb")]
            dirty_storage_keys: FxHashSet::default(),
            #[cfg(feature = "rocksdb")]
            blocks_since_flush: 0,
            #[cfg(feature = "rocksdb")]
            flush_threshold: 128,
        }
    }

    /// Check if state is available for the given block.
    ///
    /// Returns true if:
    /// - The block is the current flushed base (its state is in the on-disk trie), OR
    /// - The block number is at or before the base block (historical, already flushed), OR
    /// - A diff layer exists for the block hash (recent, in-memory).
    pub fn has_state_for_block(&self, block_hash: H256, block_number: u64) -> bool {
        block_hash == self.diff_tree.base_hash
            || block_number <= self.diff_tree.base_block
            || self.diff_tree.layers.contains_key(&block_hash)
    }

    /// Open a persistent `BinaryTrieState` from a RocksDB path.
    ///
    /// If the database already contains data (the trie root is present),
    /// the trie nodes, code store, and storage keys are loaded from it.
    /// If the database is new/empty, an empty state is returned — the
    /// caller is responsible for applying genesis.
    #[cfg(feature = "rocksdb")]
    pub fn open(path: &std::path::Path) -> Result<Self, BinaryTrieError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);

        // Limit RocksDB memory usage: 256MB block cache, small write buffers.
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_cache(&rocksdb::Cache::new_lru_cache(256 * 1024 * 1024));
        block_opts.set_cache_index_and_filter_blocks(true);
        opts.set_block_based_table_factory(&block_opts);
        opts.set_write_buffer_size(32 * 1024 * 1024); // 32MB
        opts.set_max_write_buffer_number(2);

        let db = Arc::new(
            rocksdb::DB::open(&opts, path)
                .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?,
        );

        let store = NodeStore::open(Arc::clone(&db))?;
        let root = store.load_root();
        let trie = BinaryTrie { store, root };

        let (code_store, storage_keys) = if root.is_some() {
            // Existing database — load side structures.
            let code_store = load_code_store(&db)?;
            let storage_keys = load_storage_keys(&db)?;
            (code_store, storage_keys)
        } else {
            (FxHashMap::default(), FxHashMap::default())
        };

        // The flushed root on disk IS the base for diff layers.
        let base_root = root;

        // Restore the diff tree base_hash from disk so that disk-backed
        // backward walks know where to stop and fall through to the base trie.
        let base_hash = db
            .get(META_BASE_HASH_KEY)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
            .and_then(|bytes| {
                if bytes.len() >= 32 {
                    Some(H256::from_slice(&bytes[..32]))
                } else {
                    None
                }
            })
            .unwrap_or(H256::zero());

        let mut diff_tree = DiffTree::new();
        diff_tree.base_hash = base_hash;

        Ok(Self {
            trie,
            diff_tree,
            base_root,
            code_store: Mutex::new(code_store),
            storage_keys: Mutex::new(storage_keys),
            db: Some(db),
            dirty_codes: FxHashSet::default(),
            dirty_storage_keys: FxHashSet::default(),
            blocks_since_flush: 0,
            flush_threshold: 128,
        })
    }

    /// Returns `true` if the trie contains any data (i.e. a root node exists).
    ///
    /// Used to decide whether genesis needs to be applied after `open()`.
    #[cfg(feature = "rocksdb")]
    pub fn has_data(&self) -> bool {
        self.trie.root.is_some()
    }

    /// Returns the last block number recorded by `flush()`, if any.
    #[cfg(feature = "rocksdb")]
    pub fn checkpoint_block(&self) -> Option<u64> {
        let db = self.db.as_ref()?;
        let bytes = db.get(META_BLOCK_KEY).ok()??;
        if bytes.len() < 8 {
            return None;
        }
        Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
    }

    /// Persist the current state and record `block_number` as the checkpoint.
    ///
    /// All dirty trie nodes, code entries, and storage_keys entries are written
    /// atomically in a single `WriteBatch`. On success the dirty sets are cleared.
    #[cfg(feature = "rocksdb")]
    pub fn flush(&mut self, block_number: u64, block_hash: H256) -> Result<(), BinaryTrieError> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| BinaryTrieError::StoreError("no DB configured".into()))?
            .clone();

        let mut batch = rocksdb::WriteBatch::default();

        // 1. Flush trie nodes (dirty + freed nodes, root, next_id).
        self.trie.flush_to_batch(&mut batch);

        // 2. Write dirty code_store entries (prefix 0x02 || code_hash).
        {
            let code_store = self.code_store.lock().unwrap();
            for hash in &self.dirty_codes {
                let mut key = vec![CODE_PREFIX];
                key.extend_from_slice(hash.as_bytes());
                if let Some(code) = code_store.get(hash) {
                    batch.put(&key, code.as_ref());
                } else {
                    // Code was removed — delete the entry.
                    batch.delete(&key);
                }
            }
        }

        // 3. Write dirty storage_keys entries (prefix 0x03 || address).
        {
            let storage_keys = self.storage_keys.lock().unwrap();
            for addr in &self.dirty_storage_keys {
                let mut key = vec![STORAGE_KEYS_PREFIX];
                key.extend_from_slice(addr.as_bytes());
                if let Some(keys) = storage_keys.get(addr) {
                    let mut value = Vec::with_capacity(keys.len() * 32);
                    for k in keys {
                        value.extend_from_slice(k.as_bytes());
                    }
                    batch.put(&key, &value);
                } else {
                    // Account's storage was fully cleared — delete the entry.
                    batch.delete(&key);
                }
            }
        }

        // 4. Write checkpoint block number and base block hash.
        batch.put(META_BLOCK_KEY, block_number.to_le_bytes());
        batch.put(META_BASE_HASH_KEY, block_hash.as_bytes());

        db.write(batch)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;

        // Sliding-window eviction: keep only the entries that were dirty
        // (recently modified = likely hot), drop everything else.  On a cache
        // miss, entries are reloaded from RocksDB on demand.
        {
            let mut code_store = self.code_store.lock().unwrap();
            let evicted: FxHashMap<H256, Bytes> = self
                .dirty_codes
                .iter()
                .filter_map(|h| code_store.remove_entry(h))
                .collect();
            *code_store = evicted;
        }
        {
            let mut storage_keys = self.storage_keys.lock().unwrap();
            let evicted: FxHashMap<Address, FxHashSet<H256>> = self
                .dirty_storage_keys
                .iter()
                .filter_map(|a| storage_keys.remove_entry(a))
                .collect();
            *storage_keys = evicted;
        }

        self.dirty_codes.clear();
        self.dirty_storage_keys.clear();
        self.blocks_since_flush = 0;

        // Update diff tree base: after flush, disk trie matches current root.
        self.base_root = self.trie.root;
        let cutoff = block_number.saturating_sub(self.flush_threshold);
        self.diff_tree.prune_before(cutoff);
        self.diff_tree.base_hash = block_hash;
        self.diff_tree.base_block = block_number;

        Ok(())
    }

    /// Flush to disk if the block threshold has been reached.
    ///
    /// Returns `true` if a flush was performed, `false` otherwise.
    /// Call this after each block's `apply_account_update` calls.
    #[cfg(feature = "rocksdb")]
    pub fn flush_if_needed(
        &mut self,
        block_number: u64,
        block_hash: H256,
    ) -> Result<bool, BinaryTrieError> {
        self.blocks_since_flush += 1;
        if self.blocks_since_flush >= self.flush_threshold {
            self.flush(block_number, block_hash)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// No-op when RocksDB is not enabled. Prunes old diff layers to bound memory.
    #[cfg(not(feature = "rocksdb"))]
    pub fn flush_if_needed(
        &mut self,
        block_number: u64,
        _block_hash: H256,
    ) -> Result<bool, BinaryTrieError> {
        // Issue 10: prune diff layers even without rocksdb to prevent unbounded growth.
        let cutoff = block_number.saturating_sub(128);
        self.diff_tree.prune_before(cutoff);
        Ok(false)
    }

    /// Set the flush threshold (number of blocks between disk commits).
    #[cfg(feature = "rocksdb")]
    pub fn set_flush_threshold(&mut self, threshold: u64) {
        self.flush_threshold = threshold;
    }

    /// Compute the binary trie state root via merkelization.
    ///
    /// Takes `&mut self` because computed hashes are cached back into the trie
    /// nodes for incremental reuse on subsequent calls.
    ///
    /// After computing the root, strips subtree caches from dirty nodes to
    /// reduce memory between blocks (~16KB saved per dirty StemNode).
    pub fn state_root(&mut self) -> [u8; 32] {
        let root = merkelize(&mut self.trie);
        self.trie.store.strip_dirty_subtrees();
        root
    }

    /// Read account state from the binary trie.
    ///
    /// Returns None if the account doesn't exist (no basic_data leaf).
    /// The `storage_root` field is synthesized:
    ///   - EMPTY_TRIE_HASH if the account has no tracked storage keys
    ///   - A dummy non-empty hash (H256::from_low_u64_be(1)) otherwise
    pub fn get_account_state(&self, address: &Address) -> Option<AccountState> {
        let basic_data_key = get_tree_key_for_basic_data(address);
        let basic_data = self.trie.get(basic_data_key)?;

        let (_version, _code_size, nonce, balance) = unpack_basic_data(&basic_data);

        let code_hash_key = get_tree_key_for_code_hash(address);
        let code_hash = self
            .trie
            .get(code_hash_key)
            .map(H256)
            .unwrap_or(*EMPTY_KECCACK_HASH);

        // Synthesize storage_root for LevmAccount::has_storage compatibility.
        // The VM uses storage_root != EMPTY_TRIE_HASH to detect whether an account
        // has storage. We return EMPTY_TRIE_HASH when no storage is tracked, and
        // H256(1) as a sentinel when storage exists.
        let has_storage = self.has_storage_keys(address);
        let storage_root = if has_storage {
            H256::from_low_u64_be(1)
        } else {
            *EMPTY_TRIE_HASH
        };

        Some(AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        })
    }

    /// Read a storage slot value. Returns None if unset (treated as zero).
    pub fn get_storage_slot(&self, address: &Address, key: H256) -> Option<U256> {
        let storage_key = U256::from_big_endian(key.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(address, storage_key);
        self.trie.get(tree_key).map(|v| U256::from_big_endian(&v))
    }

    /// Look up code by its keccak256 hash.
    ///
    /// Checks the in-memory cache first; on a miss, reloads from RocksDB.
    pub fn get_account_code(&self, code_hash: &H256) -> Option<Bytes> {
        {
            let cache = self.code_store.lock().unwrap();
            if let Some(code) = cache.get(code_hash) {
                return Some(code.clone());
            }
        }
        #[cfg(feature = "rocksdb")]
        if let Some(db) = self.db.as_ref() {
            let mut key = vec![CODE_PREFIX];
            key.extend_from_slice(code_hash.as_bytes());
            if let Ok(Some(bytes)) = db.get(&key) {
                let code = Bytes::copy_from_slice(&bytes);
                self.code_store
                    .lock()
                    .unwrap()
                    .insert(*code_hash, code.clone());
                return Some(code);
            }
        }
        None
    }

    // ── Diff-layer-aware reads (for historical state) ─────────────────

    /// Read account state at a specific block (identified by block hash).
    ///
    /// `H256::zero()` is a special bypass used by tests and BinaryTrieVmDb::new();
    /// it reads directly from the tip trie without consulting any diff layer.
    pub fn get_account_state_at(
        &self,
        address: &Address,
        block_hash: H256,
    ) -> Result<Option<AccountState>, BinaryTrieError> {
        if block_hash.is_zero() {
            return Ok(self.get_account_state(address));
        }
        match self.diff_tree.get_account(address, block_hash) {
            DiffLookup::Found(state) => Ok(Some(state)),
            DiffLookup::Deleted => Ok(None),
            DiffLookup::NotModified => Ok(self.get_account_state_from_base(address)),
            DiffLookup::NotInMemory => {
                self.get_account_state_from_disk_diffs(address, block_hash)
            }
        }
    }

    /// Read a storage slot at a specific block.
    ///
    /// `H256::zero()` bypasses diff layers and reads from the tip trie.
    pub fn get_storage_slot_at(
        &self,
        address: &Address,
        key: H256,
        block_hash: H256,
    ) -> Result<Option<U256>, BinaryTrieError> {
        if block_hash.is_zero() {
            return Ok(self.get_storage_slot(address, key));
        }
        match self.diff_tree.get_storage(address, key, block_hash) {
            DiffLookup::Found(val) => Ok(Some(val)),
            DiffLookup::Deleted => Ok(None),
            DiffLookup::NotModified => Ok(self.get_storage_slot_from_base(address, key)),
            DiffLookup::NotInMemory => {
                self.get_storage_slot_from_disk_diffs(address, key, block_hash)
            }
        }
    }

    /// Look up code at a specific block.
    ///
    /// `H256::zero()` bypasses diff layers and reads from the tip trie.
    pub fn get_account_code_at(
        &self,
        code_hash: &H256,
        block_hash: H256,
    ) -> Result<Option<Bytes>, BinaryTrieError> {
        if block_hash.is_zero() {
            return Ok(self.get_account_code(code_hash));
        }
        match self.diff_tree.get_code(code_hash, block_hash) {
            DiffLookup::Found(code) => Ok(Some(code)),
            // Code is never deleted in practice.
            DiffLookup::Deleted => Ok(None),
            DiffLookup::NotModified => Ok(self.get_account_code(code_hash)),
            DiffLookup::NotInMemory => {
                self.get_account_code_from_disk_diffs(code_hash, block_hash)
            }
        }
    }

    /// Read account from the base (flushed) trie.
    fn get_account_state_from_base(&self, address: &Address) -> Option<AccountState> {
        if self.base_root.is_none() {
            // No flush yet -- tip trie IS the base.
            return self.get_account_state(address);
        }

        let basic_data_key = get_tree_key_for_basic_data(address);
        let basic_data = self.trie.get_from_base(self.base_root, basic_data_key)?;

        let (_version, _code_size, nonce, balance) = unpack_basic_data(&basic_data);

        let code_hash_key = get_tree_key_for_code_hash(address);
        let code_hash = self
            .trie
            .get_from_base(self.base_root, code_hash_key)
            .map(H256)
            .unwrap_or(*EMPTY_KECCACK_HASH);

        let has_storage = self.has_storage_keys(address);
        let storage_root = if has_storage {
            H256::from_low_u64_be(1)
        } else {
            *EMPTY_TRIE_HASH
        };

        Some(AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        })
    }

    /// Read a storage slot from the base (flushed) trie.
    fn get_storage_slot_from_base(&self, address: &Address, key: H256) -> Option<U256> {
        if self.base_root.is_none() {
            return self.get_storage_slot(address, key);
        }

        let storage_key = U256::from_big_endian(key.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(address, storage_key);
        self.trie
            .get_from_base(self.base_root, tree_key)
            .map(|v| U256::from_big_endian(&v))
    }

    // ── End diff-layer-aware reads ─────────────────────────────────────

    /// Get code size from basic_data. Returns 0 if account doesn't exist.
    pub fn get_code_size(&self, address: &Address) -> u32 {
        let basic_data_key = get_tree_key_for_basic_data(address);
        match self.trie.get(basic_data_key) {
            Some(data) => {
                let (_version, code_size, _nonce, _balance) = unpack_basic_data(&data);
                code_size
            }
            None => 0,
        }
    }

    // ── Diff-layer write operations ──────────────────────────────────────

    /// Set the diff tree's base block (the state that's fully in the trie).
    /// Call after genesis init or after opening from disk.
    pub fn set_diff_base(&mut self, block_hash: H256, block_number: u64) {
        self.diff_tree.base_hash = block_hash;
        self.diff_tree.base_block = block_number;
    }

    /// Create a new diff layer for a block.
    /// Must be called before `apply_account_update_for_block`.
    pub fn begin_block(&mut self, block_hash: H256, parent_hash: H256, block_number: u64) {
        self.diff_tree
            .add_layer(block_hash, parent_hash, block_number, StateDiff::default());
    }

    /// Record an account update in the diff layer WITHOUT modifying the trie.
    /// Use this for speculative operations (payload building) where the trie
    /// shouldn't be advanced but the diff tree needs the state for future reads.
    pub fn record_diff_only(
        &mut self,
        update: &AccountUpdate,
        block_hash: H256,
    ) -> Result<(), BinaryTrieError> {
        // Build account state from the update info directly (can't read from
        // trie since it may not reflect the parent's state for speculative ops).
        let account_state = update.info.as_ref().map(|info| AccountState {
            nonce: info.nonce,
            balance: info.balance,
            code_hash: info.code_hash,
            // Conservative sentinel: the VM uses storage_root != EMPTY_TRIE_HASH
            // to decide whether to call get_storage_slot. Always returning non-empty
            // is safe (get_storage_slot handles the empty case correctly).
            storage_root: H256::from_low_u64_be(1),
        });

        let layer = self
            .diff_tree
            .layers
            .get_mut(&block_hash)
            .expect("begin_block must be called before record_diff_only");

        write_diff_layer(&mut layer.diff, update, account_state);
        Ok(())
    }

    /// Apply an account update and record it in the diff layer for `block_hash`.
    /// Call `begin_block` first.
    pub fn apply_account_update_for_block(
        &mut self,
        update: &AccountUpdate,
        block_hash: H256,
    ) -> Result<(), BinaryTrieError> {
        // 1. Apply to the trie (keeps trie at tip for state_root/get_proof).
        self.apply_account_update(update)?;

        // 2. Read post-update state BEFORE taking a mutable ref to diff_tree.
        let account_state = if !update.removed {
            self.get_account_state(&update.address)
        } else {
            None
        };

        let layer = self
            .diff_tree
            .layers
            .get_mut(&block_hash)
            .expect("begin_block must be called before apply_account_update_for_block");

        write_diff_layer(&mut layer.diff, update, account_state);
        Ok(())
    }

    // ── End diff-layer write operations ────────────────────────────────

    /// Apply a single AccountUpdate to the trie (no diff layer recording).
    pub fn apply_account_update(&mut self, update: &AccountUpdate) -> Result<(), BinaryTrieError> {
        let address = &update.address;

        // These two flags are mutually exclusive by construction in the VM:
        // removed_storage = SELFDESTRUCT + recreate, removed = fully destroyed.
        debug_assert!(
            !(update.removed_storage && update.removed),
            "removed_storage and removed should not both be true"
        );

        // Handle removed_storage (SELFDESTRUCT then recreate).
        // Must run before the removed check — see ordering comment below.
        if update.removed_storage {
            self.clear_account_storage(address)?;
        }

        // Handle full account removal.
        if update.removed {
            self.remove_account(address)?;
            return Ok(());
        }

        // Write code BEFORE account info — write_code reads old code_size from
        // basic_data to know how many old chunks to evict. write_account_info
        // overwrites basic_data with the new code_size, so it must come after.
        if let Some(ref code) = update.code {
            self.write_code(address, code)?;

            // If code changed but info wasn't provided (defensive — LEVM always
            // sends both together), update the code_hash leaf directly so the trie
            // stays consistent with code_store.
            if update.info.is_none() {
                self.trie
                    .insert(get_tree_key_for_code_hash(address), code.hash.0)?;
            }
        }

        // Apply account info changes (writes basic_data + code_hash).
        if let Some(ref info) = update.info {
            self.write_account_info(address, info, update.code.as_ref())?;
        }

        // Apply storage changes.
        for (key, value) in &update.added_storage {
            let storage_key = U256::from_big_endian(key.as_bytes());
            let tree_key = get_tree_key_for_storage_slot(address, storage_key);

            if value.is_zero() {
                // Zero means delete.
                self.trie.remove(tree_key)?;
                let mut storage_keys = self.storage_keys.lock().unwrap();
                if let Some(keys) = storage_keys.get_mut(address) {
                    keys.remove(key);
                    if keys.is_empty() {
                        storage_keys.remove(address);
                    }
                }
            } else {
                self.trie.insert(tree_key, value.to_big_endian())?;
                self.storage_keys
                    .lock()
                    .unwrap()
                    .entry(*address)
                    .or_default()
                    .insert(*key);
            }
            #[cfg(feature = "rocksdb")]
            self.dirty_storage_keys.insert(*address);
        }

        Ok(())
    }

    /// Apply genesis allocations to the trie.
    pub fn apply_genesis(
        &mut self,
        accounts: &BTreeMap<Address, GenesisAccount>,
    ) -> Result<(), BinaryTrieError> {
        for (address, genesis) in accounts {
            let code_hash = keccak(genesis.code.as_ref());
            let code_size = genesis.code.len() as u32;

            // Write basic_data.
            let basic_data = pack_basic_data(0, code_size, genesis.nonce, genesis.balance);
            self.trie
                .insert(get_tree_key_for_basic_data(address), basic_data)?;

            // Write code_hash.
            self.trie
                .insert(get_tree_key_for_code_hash(address), code_hash.0)?;

            // Write code chunks and store for fast lookup.
            if !genesis.code.is_empty() {
                let chunks = chunkify_code(&genesis.code);
                for (i, chunk) in chunks.iter().enumerate() {
                    self.trie
                        .insert(get_tree_key_for_code_chunk(address, i as u64), *chunk)?;
                }
                self.code_store
                    .lock()
                    .unwrap()
                    .insert(code_hash, genesis.code.clone());
                #[cfg(feature = "rocksdb")]
                self.dirty_codes.insert(code_hash);
            }

            // Write storage slots.
            for (slot, value) in &genesis.storage {
                if !value.is_zero() {
                    let tree_key = get_tree_key_for_storage_slot(address, *slot);
                    self.trie.insert(tree_key, value.to_big_endian())?;

                    let key_h256 = H256(slot.to_big_endian());
                    self.storage_keys
                        .lock()
                        .unwrap()
                        .entry(*address)
                        .or_default()
                        .insert(key_h256);
                    #[cfg(feature = "rocksdb")]
                    self.dirty_storage_keys.insert(*address);
                }
            }
        }
        Ok(())
    }

    /// Return sizes of the main in-memory collections for diagnostics.
    ///
    /// Returns `(clean_cache, warm, dirty_nodes, freed, code_store, storage_keys_accounts)`.
    pub fn memory_stats(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.trie.store.clean_cache_len(),
            self.trie.store.warm_len(),
            self.trie.store.dirty_len(),
            self.trie.store.freed_len(),
            self.code_store.lock().unwrap().len(),
            self.storage_keys.lock().unwrap().len(),
        )
    }

    // -------------------------------------------------------------------------
    // Witness generation
    // -------------------------------------------------------------------------

    /// Generate a binary trie execution witness from recorded state accesses.
    ///
    /// **IMPORTANT**: Call this BEFORE applying account updates for the block,
    /// so that proofs are generated against the pre-execution state root.
    /// Requires that `state_root()` was called after the previous block
    /// (so node hashes are cached for proof generation).
    ///
    /// - `block_number` / `block_hash`: identifies the block
    /// - `accessed_accounts`: map of address -> list of storage keys accessed
    /// - `accessed_codes`: set of code hashes whose bytecode was accessed
    /// - `block_headers`: RLP-encoded headers needed for BLOCKHASH opcode
    pub fn generate_witness(
        &self,
        block_number: u64,
        block_hash: H256,
        accessed_accounts: &std::collections::HashMap<Address, Vec<H256>>,
        accessed_codes: &std::collections::HashSet<H256>,
        block_headers: Vec<Vec<u8>>,
    ) -> Result<crate::witness::BinaryTrieWitness, BinaryTrieError> {
        use crate::key_mapping::{
            get_tree_key_for_basic_data, get_tree_key_for_code_hash,
            get_tree_key_for_storage_slot, unpack_basic_data,
        };
        use crate::witness::{
            AccountWitnessEntry, BinaryTrieWitness, CodeWitnessEntry, ProofEntry,
            StorageWitnessEntry,
        };
        use ethrex_common::{U256, constants::EMPTY_KECCACK_HASH};

        // Read the cached root hash -- this is the PRE-execution state root
        // since the caller has not yet applied this block's updates.
        let pre_state_root = match self.trie.root {
            None => crate::merkle::ZERO_HASH,
            Some(root_id) => {
                let node = self.trie.store.get(root_id)?;
                match node {
                    crate::node::Node::Internal(internal) => internal
                        .cached_hash
                        .ok_or(BinaryTrieError::ProofRequiresMerkelization)?,
                    crate::node::Node::Stem(stem_node) => stem_node
                        .cached_hash
                        .ok_or(BinaryTrieError::ProofRequiresMerkelization)?,
                }
            }
        };

        let mut account_proofs = Vec::new();
        let mut storage_proofs = Vec::new();

        for (address, storage_keys) in accessed_accounts {
            let basic_data_key = get_tree_key_for_basic_data(address);
            let code_hash_key = get_tree_key_for_code_hash(address);

            let bd_proof = self.trie.get_proof(basic_data_key)?;
            let ch_proof = self.trie.get_proof(code_hash_key)?;

            // Extract pre-state values from the proof leaves.
            let (balance, nonce, code_hash) = if let Some(ref basic_data) = bd_proof.value {
                let (_version, _code_size, nonce, balance) = unpack_basic_data(basic_data);
                let code_hash = ch_proof
                    .value
                    .map(H256)
                    .unwrap_or(*EMPTY_KECCACK_HASH);
                (balance, nonce, code_hash)
            } else {
                (U256::zero(), 0, *EMPTY_KECCACK_HASH)
            };

            account_proofs.push(AccountWitnessEntry {
                address: *address,
                balance,
                nonce,
                code_hash,
                basic_data_proof: ProofEntry {
                    siblings: bd_proof.siblings,
                    stem_depth: bd_proof.stem_depth,
                    value: bd_proof.value,
                },
                code_hash_proof: ProofEntry {
                    siblings: ch_proof.siblings,
                    stem_depth: ch_proof.stem_depth,
                    value: ch_proof.value,
                },
            });

            for slot_key in storage_keys {
                let storage_key_u256 = U256::from_big_endian(slot_key.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(address, storage_key_u256);
                let proof = self.trie.get_proof(tree_key)?;

                let value = proof
                    .value
                    .map(|v| U256::from_big_endian(&v))
                    .unwrap_or_default();

                storage_proofs.push(StorageWitnessEntry {
                    address: *address,
                    slot: *slot_key,
                    value,
                    proof: ProofEntry {
                        siblings: proof.siblings,
                        stem_depth: proof.stem_depth,
                        value: proof.value,
                    },
                });
            }
        }

        let mut codes = Vec::new();
        for code_hash in accessed_codes {
            if let Some(bytecode) = self.get_account_code(code_hash) {
                codes.push(CodeWitnessEntry {
                    code_hash: *code_hash,
                    bytecode: bytecode.to_vec(),
                });
            }
        }

        Ok(BinaryTrieWitness {
            block_number,
            block_hash,
            pre_state_root,
            account_proofs,
            storage_proofs,
            codes,
            block_headers,
        })
    }

    // -------------------------------------------------------------------------
    // Trie reconstruction (for historical proof generation)
    // -------------------------------------------------------------------------

    /// Create a temporary `BinaryTrieState` at a specific historical block
    /// by cloning the base trie from RocksDB and replaying persisted diffs.
    ///
    /// The returned state has a trie at the post-state of `target_parent_hash`
    /// (i.e., the pre-state of the block whose parent is `target_parent_hash`).
    /// Call `state_root()` on the result to merkelise, then `generate_witness()`.
    ///
    /// NOTE: When periodic snapshots are implemented, this method should load
    /// from the nearest snapshot instead of always starting from the flush-point
    /// base trie.
    #[cfg(feature = "rocksdb")]
    pub fn reconstruct_at_block(
        &self,
        target_parent_hash: H256,
    ) -> Result<BinaryTrieState, BinaryTrieError> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| BinaryTrieError::StoreError("no DB configured".into()))?;

        // Start from the base trie on disk (the flush-point snapshot).
        let store = NodeStore::open(Arc::clone(db))?;
        let root = store.load_root();
        let trie = BinaryTrie { store, root };

        let code_store = load_code_store(db)?;
        let storage_keys = load_storage_keys(db)?;

        let mut temp_state = BinaryTrieState {
            trie,
            diff_tree: DiffTree::new(),
            base_root: root,
            code_store: Mutex::new(code_store),
            storage_keys: Mutex::new(storage_keys),
            db: Some(Arc::clone(db)),
            dirty_codes: FxHashSet::default(),
            dirty_storage_keys: FxHashSet::default(),
            blocks_since_flush: 0,
            flush_threshold: u64::MAX, // never flush the temp state
        };

        // Collect the chain of diffs from base to target_parent_hash.
        // Walk backward from target_parent_hash to base_hash, collecting
        // diffs in reverse order, then apply them forward.
        let mut diff_chain = Vec::new();
        let mut current = target_parent_hash;

        const MAX_REPLAY: u64 = 100_000;
        for _ in 0..MAX_REPLAY {
            if current == self.diff_tree.base_hash || current.is_zero() {
                break;
            }
            // Check in-memory diffs first.
            if let Some(node) = self.diff_tree.layers.get(&current) {
                diff_chain.push((current, &node.diff));
                current = node.parent_hash;
                continue;
            }
            // Not in memory -- stop here. The remaining diffs would need
            // to be loaded from disk, but we can't hold references to them
            // across iterations. Break and load from disk below.
            break;
        }

        // If we didn't reach the base via in-memory diffs, load from disk.
        if current != self.diff_tree.base_hash && !current.is_zero() {
            let mut disk_diffs = Vec::new();
            for _ in 0..MAX_REPLAY {
                if current == self.diff_tree.base_hash || current.is_zero() {
                    break;
                }
                let (parent_hash, _block_number, diff) =
                    self.load_diff_from_disk(current)?.ok_or_else(|| {
                        BinaryTrieError::StoreError(format!(
                            "missing diff record for block {current:?} during reconstruction"
                        ))
                    })?;
                disk_diffs.push(diff);
                current = parent_hash;
            }
            // Apply disk diffs in reverse (oldest first).
            for diff in disk_diffs.into_iter().rev() {
                temp_state.apply_diff(&diff)?;
            }
        }

        // Apply in-memory diffs in reverse (oldest first).
        for (_hash, diff) in diff_chain.into_iter().rev() {
            temp_state.apply_diff(diff)?;
        }

        Ok(temp_state)
    }

    /// Trie reconstruction is not available without persistence.
    /// In-memory trie states have no persisted diffs to replay from.
    #[cfg(not(feature = "rocksdb"))]
    pub fn reconstruct_at_block(
        &self,
        _target_parent_hash: H256,
    ) -> Result<BinaryTrieState, BinaryTrieError> {
        Err(BinaryTrieError::StoreError(
            "trie reconstruction requires persistent storage (rocksdb)".into(),
        ))
    }

    /// Apply a `StateDiff` directly to the trie.
    ///
    /// This writes the post-state values from the diff into the trie,
    /// advancing it by one block. Used for trie reconstruction from
    /// persisted diffs.
    fn apply_diff(&mut self, diff: &StateDiff) -> Result<(), BinaryTrieError> {
        use crate::key_mapping::{
            chunkify_code, get_tree_key_for_basic_data, get_tree_key_for_code_hash,
            get_tree_key_for_code_chunk, get_tree_key_for_storage_slot, pack_basic_data,
        };

        // Apply storage_cleared first (remove all tracked storage for those addresses).
        for address in &diff.storage_cleared {
            self.clear_account_storage(address)?;
        }

        // Apply account changes.
        for (address, account_state_opt) in &diff.accounts {
            match account_state_opt {
                Some(state) => {
                    // Compute code_size from the code store if code was deployed.
                    let code_size = diff
                        .code
                        .iter()
                        .find(|(_, _)| {
                            // Check if this code belongs to this account.
                            state.code_hash != *EMPTY_KECCACK_HASH
                        })
                        .and_then(|(hash, code)| {
                            if *hash == state.code_hash {
                                Some(code.len() as u32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| self.get_code_size(address));

                    let basic_data =
                        pack_basic_data(0, code_size, state.nonce, state.balance);
                    self.trie
                        .insert(get_tree_key_for_basic_data(address), basic_data)?;
                    self.trie
                        .insert(get_tree_key_for_code_hash(address), state.code_hash.0)?;
                }
                None => {
                    // Account deleted.
                    self.remove_account(address)?;
                }
            }
        }

        // Apply code deployments.
        for (code_hash, bytecode) in &diff.code {
            let chunks = chunkify_code(bytecode);
            // Find the address that deployed this code.
            for (address, state_opt) in &diff.accounts {
                if let Some(state) = state_opt {
                    if state.code_hash == *code_hash {
                        for (i, chunk) in chunks.iter().enumerate() {
                            self.trie.insert(
                                get_tree_key_for_code_chunk(address, i as u64),
                                *chunk,
                            )?;
                        }
                        break;
                    }
                }
            }
            self.code_store
                .lock()
                .unwrap()
                .insert(*code_hash, bytecode.clone());
        }

        // Apply storage changes.
        for ((address, key), value_opt) in &diff.storage {
            let storage_key = U256::from_big_endian(key.as_bytes());
            let tree_key = get_tree_key_for_storage_slot(address, storage_key);
            match value_opt {
                Some(value) => {
                    self.trie.insert(tree_key, value.to_big_endian())?;
                    self.storage_keys
                        .lock()
                        .unwrap()
                        .entry(*address)
                        .or_default()
                        .insert(*key);
                }
                None => {
                    self.trie.remove(tree_key)?;
                    let mut sk = self.storage_keys.lock().unwrap();
                    if let Some(keys) = sk.get_mut(address) {
                        keys.remove(key);
                        if keys.is_empty() {
                            sk.remove(address);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    /// Check whether `address` has any tracked storage keys.
    ///
    /// Checks in-memory cache first; on a miss, reloads from RocksDB.
    fn has_storage_keys(&self, address: &Address) -> bool {
        {
            let cache = self.storage_keys.lock().unwrap();
            if let Some(keys) = cache.get(address) {
                return !keys.is_empty();
            }
        }
        self.reload_storage_keys(address)
    }

    /// Reload storage_keys for a single address from RocksDB.
    /// Returns true if the address has non-empty storage keys.
    #[cfg(feature = "rocksdb")]
    fn reload_storage_keys(&self, address: &Address) -> bool {
        let Some(db) = self.db.as_ref() else {
            return false;
        };
        let mut key = vec![STORAGE_KEYS_PREFIX];
        key.extend_from_slice(address.as_bytes());
        if let Ok(Some(value)) = db.get(&key) {
            if !value.is_empty() {
                let mut keys = FxHashSet::default();
                let mut offset = 0usize;
                while offset + 32 <= value.len() {
                    keys.insert(H256::from_slice(&value[offset..offset + 32]));
                    offset += 32;
                }
                let has = !keys.is_empty();
                self.storage_keys.lock().unwrap().insert(*address, keys);
                return has;
            }
        }
        false
    }

    #[cfg(not(feature = "rocksdb"))]
    fn reload_storage_keys(&self, _address: &Address) -> bool {
        false
    }

    /// Write basic_data leaf for an account.
    ///
    /// If new code is being deployed (code is Some), uses its length for code_size.
    /// Otherwise preserves the existing code_size from the trie.
    fn write_account_info(
        &mut self,
        address: &Address,
        info: &AccountInfo,
        new_code: Option<&Code>,
    ) -> Result<(), BinaryTrieError> {
        let code_size = new_code
            .map(|c| c.bytecode.len() as u32)
            .unwrap_or_else(|| self.get_code_size(address));

        let basic_data = pack_basic_data(0, code_size, info.nonce, info.balance);
        self.trie
            .insert(get_tree_key_for_basic_data(address), basic_data)?;

        self.trie
            .insert(get_tree_key_for_code_hash(address), info.code_hash.0)?;

        Ok(())
    }

    /// Write code: chunkify into trie leaves + store in code_store.
    fn write_code(&mut self, address: &Address, code: &Code) -> Result<(), BinaryTrieError> {
        // Remove old code chunks if code_size changed.
        let old_code_size = self.get_code_size(address);
        if old_code_size > 0 {
            let old_num_chunks = (old_code_size as u64).div_ceil(31);
            let new_num_chunks = if code.bytecode.is_empty() {
                0
            } else {
                (code.bytecode.len() as u64).div_ceil(31)
            };
            // Remove chunks that won't be overwritten by the new code.
            for chunk_id in new_num_chunks..old_num_chunks {
                self.trie
                    .remove(get_tree_key_for_code_chunk(address, chunk_id))?;
            }
        }

        // Write new code chunks.
        let chunks = chunkify_code(&code.bytecode);
        for (i, chunk) in chunks.iter().enumerate() {
            self.trie
                .insert(get_tree_key_for_code_chunk(address, i as u64), *chunk)?;
        }

        // Store in code_store for fast lookup.
        self.code_store
            .lock()
            .unwrap()
            .insert(code.hash, code.bytecode.clone());
        #[cfg(feature = "rocksdb")]
        self.dirty_codes.insert(code.hash);

        Ok(())
    }

    /// Remove all state for an account (basic_data, code_hash, code chunks, storage).
    fn remove_account(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        // Read code_size BEFORE removing basic_data — needed to know chunk count.
        let code_size = self.get_code_size(address);

        // Remove basic_data and code_hash leaves.
        self.trie.remove(get_tree_key_for_basic_data(address))?;
        self.trie.remove(get_tree_key_for_code_hash(address))?;

        // Remove code chunks.
        if code_size > 0 {
            let num_chunks = (code_size as u64).div_ceil(31);
            for chunk_id in 0..num_chunks {
                self.trie
                    .remove(get_tree_key_for_code_chunk(address, chunk_id))?;
            }
        }

        // Remove all tracked storage.
        self.clear_account_storage(address)?;

        // Note: code_store entries are keyed by hash. We intentionally do not remove
        // them on account deletion — other accounts may share the same bytecode.

        Ok(())
    }

    /// Clear all storage slots for an account using the tracked storage_keys.
    fn clear_account_storage(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        // Ensure storage_keys are loaded if they were evicted.
        if !self.storage_keys.lock().unwrap().contains_key(address) {
            self.reload_storage_keys(address);
        }
        let keys = self.storage_keys.lock().unwrap().remove(address);
        if let Some(keys) = keys {
            for key in keys {
                let storage_key = U256::from_big_endian(key.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(address, storage_key);
                self.trie.remove(tree_key)?;
            }
            // Mark the address as dirty (entry removed from storage_keys).
            #[cfg(feature = "rocksdb")]
            self.dirty_storage_keys.insert(*address);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RocksDB helper loaders (used by `open`)
// ---------------------------------------------------------------------------

/// Load the code_store from RocksDB by iterating over keys with prefix 0x02.
#[cfg(feature = "rocksdb")]
fn load_code_store(db: &rocksdb::DB) -> Result<FxHashMap<H256, Bytes>, BinaryTrieError> {
    let prefix = [CODE_PREFIX];
    let mut map = FxHashMap::default();
    // Use a forward iterator starting at the prefix byte, with manual stop.
    // We don't use prefix_iterator because no prefix extractor is configured.
    let iter = db.iterator(rocksdb::IteratorMode::From(
        &prefix,
        rocksdb::Direction::Forward,
    ));
    for item in iter {
        let (key, value) = item.map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        if key.first() != Some(&CODE_PREFIX) {
            break;
        }
        if key.len() < 1 + 32 {
            continue;
        }
        let hash = H256::from_slice(&key[1..33]);
        map.insert(hash, Bytes::copy_from_slice(&value));
    }
    Ok(map)
}

/// Load the storage_keys from RocksDB by iterating over keys with prefix 0x03.
#[cfg(feature = "rocksdb")]
fn load_storage_keys(
    db: &rocksdb::DB,
) -> Result<FxHashMap<Address, FxHashSet<H256>>, BinaryTrieError> {
    let prefix = [STORAGE_KEYS_PREFIX];
    let mut map: FxHashMap<Address, FxHashSet<H256>> = FxHashMap::default();
    let iter = db.iterator(rocksdb::IteratorMode::From(
        &prefix,
        rocksdb::Direction::Forward,
    ));
    for item in iter {
        let (key, value) = item.map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        if key.first() != Some(&STORAGE_KEYS_PREFIX) {
            break;
        }
        if key.len() < 1 + 20 {
            continue;
        }
        let addr = Address::from_slice(&key[1..21]);
        let mut keys = FxHashSet::default();
        let mut offset = 0usize;
        while offset + 32 <= value.len() {
            let h = H256::from_slice(&value[offset..offset + 32]);
            keys.insert(h);
            offset += 32;
        }
        map.insert(addr, keys);
    }
    Ok(map)
}

impl Default for BinaryTrieState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Disk-backed diff persistence (rocksdb feature only)
// ---------------------------------------------------------------------------

impl BinaryTrieState {
    /// Persist the diff layer for `block_hash` to RocksDB.
    ///
    /// Call after all `apply_account_update_for_block` calls for the block.
    /// This is crash-safe: each block's diff is written individually rather
    /// than waiting until flush.
    #[cfg(feature = "rocksdb")]
    pub fn persist_diff(&self, block_hash: H256) -> Result<(), BinaryTrieError> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| BinaryTrieError::StoreError("no DB configured".into()))?;
        let layer = self.diff_tree.layers.get(&block_hash).ok_or_else(|| {
            BinaryTrieError::StoreError(format!("no diff layer for block {block_hash:?}"))
        })?;
        let bytes =
            serialize_diff_record(&layer.parent_hash, layer.block_number, &layer.diff);
        let mut key = vec![DIFF_PREFIX];
        key.extend_from_slice(block_hash.as_bytes());
        db.put(&key, &bytes)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        Ok(())
    }

    #[cfg(not(feature = "rocksdb"))]
    pub fn persist_diff(&self, _block_hash: H256) -> Result<(), BinaryTrieError> {
        Ok(())
    }

    /// Load a persisted diff record for a block hash from RocksDB.
    #[cfg(feature = "rocksdb")]
    fn load_diff_from_disk(
        &self,
        block_hash: H256,
    ) -> Result<Option<(H256, u64, StateDiff)>, BinaryTrieError> {
        let db = match self.db.as_ref() {
            Some(db) => db,
            None => return Ok(None),
        };
        let mut key = vec![DIFF_PREFIX];
        key.extend_from_slice(block_hash.as_bytes());
        match db.get(&key).map_err(|e| BinaryTrieError::StoreError(e.to_string()))? {
            Some(bytes) => Ok(Some(deserialize_diff_record(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Walk persisted diffs backward from `block_hash` to find the account state.
    #[cfg(feature = "rocksdb")]
    fn get_account_state_from_disk_diffs(
        &self,
        address: &Address,
        block_hash: H256,
    ) -> Result<Option<AccountState>, BinaryTrieError> {
        let mut current = block_hash;
        const MAX_WALK: u64 = 100_000;
        for _ in 0..MAX_WALK {
            if current == self.diff_tree.base_hash {
                return Ok(self.get_account_state_from_base(address));
            }
            // Check in-memory first (walk may re-enter the in-memory window).
            if let Some(node) = self.diff_tree.layers.get(&current) {
                match node.diff.accounts.get(address) {
                    Some(Some(state)) => return Ok(Some(state.clone())),
                    Some(None) => return Ok(None),
                    None => {
                        current = node.parent_hash;
                        continue;
                    }
                }
            }
            // Load from disk.
            let (parent_hash, _block_number, diff) =
                match self.load_diff_from_disk(current)? {
                    Some(record) => record,
                    None => {
                        return Err(BinaryTrieError::StoreError(format!(
                            "missing diff record for block {current:?}"
                        )));
                    }
                };
            match diff.accounts.get(address) {
                Some(Some(state)) => return Ok(Some(state.clone())),
                Some(None) => return Ok(None),
                None => current = parent_hash,
            }
        }
        Err(BinaryTrieError::StoreError(
            "exceeded max walk steps in disk diff lookup".into(),
        ))
    }

    #[cfg(not(feature = "rocksdb"))]
    fn get_account_state_from_disk_diffs(
        &self,
        _address: &Address,
        _block_hash: H256,
    ) -> Result<Option<AccountState>, BinaryTrieError> {
        Ok(None)
    }

    /// Walk persisted diffs backward from `block_hash` to find the storage slot.
    #[cfg(feature = "rocksdb")]
    fn get_storage_slot_from_disk_diffs(
        &self,
        address: &Address,
        key: H256,
        block_hash: H256,
    ) -> Result<Option<U256>, BinaryTrieError> {
        let mut current = block_hash;
        const MAX_WALK: u64 = 100_000;
        for _ in 0..MAX_WALK {
            if current == self.diff_tree.base_hash {
                return Ok(self.get_storage_slot_from_base(address, key));
            }
            // Check in-memory first.
            if let Some(node) = self.diff_tree.layers.get(&current) {
                match node.diff.storage.get(&(*address, key)) {
                    Some(Some(val)) => return Ok(Some(*val)),
                    Some(None) => return Ok(None),
                    None => {
                        if node.diff.storage_cleared.contains(address) {
                            return Ok(None);
                        }
                        current = node.parent_hash;
                        continue;
                    }
                }
            }
            // Load from disk.
            let (parent_hash, _block_number, diff) =
                match self.load_diff_from_disk(current)? {
                    Some(record) => record,
                    None => {
                        return Err(BinaryTrieError::StoreError(format!(
                            "missing diff record for block {current:?}"
                        )));
                    }
                };
            match diff.storage.get(&(*address, key)) {
                Some(Some(val)) => return Ok(Some(*val)),
                Some(None) => return Ok(None),
                None => {
                    if diff.storage_cleared.contains(address) {
                        return Ok(None);
                    }
                    current = parent_hash;
                }
            }
        }
        Err(BinaryTrieError::StoreError(
            "exceeded max walk steps in disk diff lookup".into(),
        ))
    }

    #[cfg(not(feature = "rocksdb"))]
    fn get_storage_slot_from_disk_diffs(
        &self,
        _address: &Address,
        _key: H256,
        _block_hash: H256,
    ) -> Result<Option<U256>, BinaryTrieError> {
        Ok(None)
    }

    /// Walk persisted diffs backward from `block_hash` to find code.
    ///
    /// Code is content-addressed and never deleted, so if not found in diffs,
    /// we fall back to the code store (which includes all ever-deployed code).
    #[cfg(feature = "rocksdb")]
    fn get_account_code_from_disk_diffs(
        &self,
        code_hash: &H256,
        block_hash: H256,
    ) -> Result<Option<Bytes>, BinaryTrieError> {
        // Code is content-addressed and immutable. If we can find it anywhere
        // (diffs or code store), it's the right code. We walk diffs only to
        // find recently-deployed code that may not yet be in the code store.
        let mut current = block_hash;
        const MAX_WALK: u64 = 100_000;
        for _ in 0..MAX_WALK {
            if current == self.diff_tree.base_hash {
                return Ok(self.get_account_code(code_hash));
            }
            // Check in-memory first.
            if let Some(node) = self.diff_tree.layers.get(&current) {
                if let Some(code) = node.diff.code.get(code_hash) {
                    return Ok(Some(code.clone()));
                }
                current = node.parent_hash;
                continue;
            }
            // Load from disk.
            let (parent_hash, _block_number, diff) =
                match self.load_diff_from_disk(current)? {
                    Some(record) => record,
                    None => return Ok(self.get_account_code(code_hash)),
                };
            if let Some(code) = diff.code.get(code_hash) {
                return Ok(Some(code.clone()));
            }
            current = parent_hash;
        }
        Ok(self.get_account_code(code_hash))
    }

    #[cfg(not(feature = "rocksdb"))]
    fn get_account_code_from_disk_diffs(
        &self,
        _code_hash: &H256,
        _block_hash: H256,
    ) -> Result<Option<Bytes>, BinaryTrieError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// StateDiff serialization (rocksdb feature only)
// ---------------------------------------------------------------------------

/// Serialize a `(parent_hash, block_number, StateDiff)` triple into a compact
/// binary format for on-disk storage.
///
/// Format:
/// ```text
/// [parent_hash: 32 bytes]
/// [block_number: u64 LE]
/// [accounts_count: u32 LE]
///   for each account:
///     [address: 20 bytes]
///     [tag: u8]  -- 0x00 = deleted, 0x01 = present
///     if present: [nonce: u64 LE][balance: 32 bytes BE][storage_root: 32 bytes][code_hash: 32 bytes]
/// [storage_count: u32 LE]
///   for each storage entry:
///     [address: 20 bytes][key: 32 bytes][tag: u8]  -- 0x00 = deleted, 0x01 = present
///     if present: [value: 32 bytes BE]
/// [code_count: u32 LE]
///   for each code entry: [code_hash: 32 bytes][code_len: u32 LE][code_bytes]
/// [storage_cleared_count: u32 LE]
///   for each cleared address: [address: 20 bytes]
/// ```
#[cfg(feature = "rocksdb")]
fn serialize_diff_record(parent_hash: &H256, block_number: u64, diff: &StateDiff) -> Vec<u8> {
    let mut buf = Vec::new();

    // parent_hash + block_number
    buf.extend_from_slice(parent_hash.as_bytes());
    buf.extend_from_slice(&block_number.to_le_bytes());

    // accounts
    buf.extend_from_slice(&(diff.accounts.len() as u32).to_le_bytes());
    for (addr, maybe_state) in &diff.accounts {
        buf.extend_from_slice(addr.as_bytes());
        match maybe_state {
            None => buf.push(0x00),
            Some(state) => {
                buf.push(0x01);
                buf.extend_from_slice(&state.nonce.to_le_bytes());
                buf.extend_from_slice(&state.balance.to_big_endian());
                buf.extend_from_slice(state.storage_root.as_bytes());
                buf.extend_from_slice(state.code_hash.as_bytes());
            }
        }
    }

    // storage slots
    buf.extend_from_slice(&(diff.storage.len() as u32).to_le_bytes());
    for ((addr, key), maybe_val) in &diff.storage {
        buf.extend_from_slice(addr.as_bytes());
        buf.extend_from_slice(key.as_bytes());
        match maybe_val {
            None => buf.push(0x00),
            Some(val) => {
                buf.push(0x01);
                buf.extend_from_slice(&val.to_big_endian());
            }
        }
    }

    // code entries
    buf.extend_from_slice(&(diff.code.len() as u32).to_le_bytes());
    for (hash, code) in &diff.code {
        buf.extend_from_slice(hash.as_bytes());
        buf.extend_from_slice(&(code.len() as u32).to_le_bytes());
        buf.extend_from_slice(code);
    }

    // storage_cleared addresses
    buf.extend_from_slice(&(diff.storage_cleared.len() as u32).to_le_bytes());
    for addr in &diff.storage_cleared {
        buf.extend_from_slice(addr.as_bytes());
    }

    buf
}

/// Deserialize bytes written by `serialize_diff_record`.
///
/// Returns `(parent_hash, block_number, StateDiff)`.
#[cfg(feature = "rocksdb")]
fn deserialize_diff_record(
    bytes: &[u8],
) -> Result<(H256, u64, StateDiff), BinaryTrieError> {
    let mut offset = 0usize;

    macro_rules! read_bytes {
        ($n:expr) => {{
            if offset + $n > bytes.len() {
                return Err(BinaryTrieError::DeserializationError(
                    "unexpected end of diff record".into(),
                ));
            }
            let slice = &bytes[offset..offset + $n];
            offset += $n;
            slice
        }};
    }

    macro_rules! read_u32_le {
        () => {{
            let b = read_bytes!(4);
            u32::from_le_bytes(b.try_into().unwrap())
        }};
    }

    macro_rules! read_u64_le {
        () => {{
            let b = read_bytes!(8);
            u64::from_le_bytes(b.try_into().unwrap())
        }};
    }

    let parent_hash = H256::from_slice(read_bytes!(32));
    let block_number = read_u64_le!();

    // accounts
    let accounts_count = read_u32_le!();
    let mut accounts: FxHashMap<Address, Option<AccountState>> =
        FxHashMap::with_capacity_and_hasher(accounts_count as usize, Default::default());
    for _ in 0..accounts_count {
        let addr = Address::from_slice(read_bytes!(20));
        let tag = read_bytes!(1)[0];
        let maybe_state = match tag {
            0x00 => None,
            0x01 => {
                let nonce = read_u64_le!();
                let balance = U256::from_big_endian(read_bytes!(32));
                let storage_root = H256::from_slice(read_bytes!(32));
                let code_hash = H256::from_slice(read_bytes!(32));
                Some(AccountState {
                    nonce,
                    balance,
                    storage_root,
                    code_hash,
                })
            }
            other => {
                return Err(BinaryTrieError::DeserializationError(format!(
                    "invalid account tag: {other:#x}"
                )));
            }
        };
        accounts.insert(addr, maybe_state);
    }

    // storage slots
    let storage_count = read_u32_le!();
    let mut storage: FxHashMap<(Address, H256), Option<U256>> =
        FxHashMap::with_capacity_and_hasher(storage_count as usize, Default::default());
    for _ in 0..storage_count {
        let addr = Address::from_slice(read_bytes!(20));
        let key = H256::from_slice(read_bytes!(32));
        let tag = read_bytes!(1)[0];
        let maybe_val = match tag {
            0x00 => None,
            0x01 => Some(U256::from_big_endian(read_bytes!(32))),
            other => {
                return Err(BinaryTrieError::DeserializationError(format!(
                    "invalid storage tag: {other:#x}"
                )));
            }
        };
        storage.insert((addr, key), maybe_val);
    }

    // code entries
    let code_count = read_u32_le!();
    let mut code: FxHashMap<H256, Bytes> =
        FxHashMap::with_capacity_and_hasher(code_count as usize, Default::default());
    for _ in 0..code_count {
        let hash = H256::from_slice(read_bytes!(32));
        let code_len = read_u32_le!() as usize;
        let code_bytes = Bytes::copy_from_slice(read_bytes!(code_len));
        code.insert(hash, code_bytes);
    }

    // storage_cleared
    let cleared_count = read_u32_le!();
    let mut storage_cleared: FxHashSet<Address> =
        FxHashSet::with_capacity_and_hasher(cleared_count as usize, Default::default());
    for _ in 0..cleared_count {
        let addr = Address::from_slice(read_bytes!(20));
        storage_cleared.insert(addr);
    }

    Ok((
        parent_hash,
        block_number,
        StateDiff {
            accounts,
            storage,
            code,
            storage_cleared,
        },
    ))
}

/// Write an account update into a diff layer.
/// `account_state` is the post-update account state (from trie or constructed).
/// Handles removed accounts, cleared storage, code, and storage slots.
fn write_diff_layer(
    diff: &mut StateDiff,
    update: &AccountUpdate,
    account_state: Option<AccountState>,
) {
    let addr = update.address;

    if update.removed {
        diff.accounts.insert(addr, None);
        diff.storage_cleared.insert(addr);
        return;
    }

    if update.removed_storage {
        diff.storage_cleared.insert(addr);
        // Always record the account when storage is cleared so that
        // account-state and storage-state queries are consistent.
        if account_state.is_some() {
            diff.accounts.insert(addr, account_state.clone());
        }
    }

    if let Some(info) = &update.info {
        diff.accounts.insert(addr, account_state);
        if let Some(code) = &update.code {
            diff.code.insert(info.code_hash, code.bytecode.clone());
        }
    }

    for (key, value) in &update.added_storage {
        if value.is_zero() {
            diff.storage.insert((addr, *key), None);
        } else {
            diff.storage.insert((addr, *key), Some(*value));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bytes::Bytes;
    use ethrex_common::{
        Address, H256, U256,
        constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
        types::{AccountInfo, AccountUpdate, Code, GenesisAccount},
        utils::keccak,
    };

    use super::BinaryTrieState;

    fn make_address(b: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = b;
        Address::from(a)
    }

    fn make_genesis_eoa(balance: u64, nonce: u64) -> GenesisAccount {
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::from(balance),
            nonce,
        }
    }

    fn make_genesis_contract(balance: u64, code: Bytes) -> GenesisAccount {
        GenesisAccount {
            code,
            storage: BTreeMap::new(),
            balance: U256::from(balance),
            nonce: 1,
        }
    }

    // 1. Empty state has zero root.
    #[test]
    fn test_new_state_root_is_zero() {
        let mut state = BinaryTrieState::new();
        assert_eq!(state.state_root(), [0u8; 32]);
    }

    // 2. Non-existent account returns None.
    #[test]
    fn test_get_nonexistent_account() {
        let mut state = BinaryTrieState::new();
        assert!(state.get_account_state(&make_address(1)).is_none());
    }

    // 3. Genesis with a single funded EOA; verify read-back.
    #[test]
    fn test_apply_genesis_single_account() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0xAB);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(1_000_000, 5));
        state.apply_genesis(&accounts).unwrap();

        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.balance, U256::from(1_000_000u64));
        assert_eq!(account_state.nonce, 5);
        assert_eq!(account_state.code_hash, *EMPTY_KECCACK_HASH);
        assert_eq!(account_state.storage_root, *EMPTY_TRIE_HASH);
    }

    // 4. Genesis with contract; verify code_hash and code retrieval.
    #[test]
    fn test_apply_genesis_with_code() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x01);
        let bytecode = Bytes::from(vec![0x60u8, 0x00, 0x56]); // PUSH1 0x00 JUMP
        let expected_hash = keccak(bytecode.as_ref());

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_contract(500, bytecode.clone()));
        state.apply_genesis(&accounts).unwrap();

        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.code_hash, expected_hash);

        let retrieved = state.get_account_code(&expected_hash).unwrap();
        assert_eq!(retrieved, bytecode);
    }

    // 5. Genesis with storage slots; verify read-back.
    #[test]
    fn test_apply_genesis_with_storage() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x02);

        let mut storage = BTreeMap::new();
        storage.insert(U256::from(0u64), U256::from(42u64));
        storage.insert(U256::from(1u64), U256::from(99u64));

        let genesis_account = GenesisAccount {
            code: Bytes::new(),
            storage,
            balance: U256::from(100u64),
            nonce: 0,
        };

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, genesis_account);
        state.apply_genesis(&accounts).unwrap();

        let slot0 = state
            .get_storage_slot(&addr, H256(U256::from(0u64).to_big_endian()))
            .unwrap();
        assert_eq!(slot0, U256::from(42u64));

        let slot1 = state
            .get_storage_slot(&addr, H256(U256::from(1u64).to_big_endian()))
            .unwrap();
        assert_eq!(slot1, U256::from(99u64));
    }

    // 6. apply_account_update with balance/nonce change.
    #[test]
    fn test_apply_account_update_balance_change() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x10);

        // Genesis: 100 ETH, nonce 0.
        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(100, 0));
        state.apply_genesis(&accounts).unwrap();

        // Update: 200 ETH, nonce 1.
        let mut update = AccountUpdate::new(addr);
        update.info = Some(AccountInfo {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: U256::from(200u64),
            nonce: 1,
        });
        state.apply_account_update(&update).unwrap();

        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.balance, U256::from(200u64));
        assert_eq!(account_state.nonce, 1);
    }

    // 7. apply_account_update deploys code.
    #[test]
    fn test_apply_account_update_deploy_code() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x11);

        // Genesis: empty account.
        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(50, 0));
        state.apply_genesis(&accounts).unwrap();

        let bytecode = Bytes::from(vec![0x5Bu8; 62]); // 62 JUMPDEST bytes → 2 chunks
        let code = Code::from_bytecode(bytecode.clone());
        let code_hash = code.hash;

        let mut update = AccountUpdate::new(addr);
        update.info = Some(AccountInfo {
            code_hash,
            balance: U256::from(50u64),
            nonce: 1,
        });
        update.code = Some(code);
        state.apply_account_update(&update).unwrap();

        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.code_hash, code_hash);

        let retrieved = state.get_account_code(&code_hash).unwrap();
        assert_eq!(retrieved, bytecode);

        assert_eq!(state.get_code_size(&addr), 62);
    }

    // 8. apply_account_update writes storage.
    #[test]
    fn test_apply_account_update_storage_write() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x12);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(0, 0));
        state.apply_genesis(&accounts).unwrap();

        let slot_key = H256(U256::from(5u64).to_big_endian());
        let mut update = AccountUpdate::new(addr);
        update.added_storage.insert(slot_key, U256::from(777u64));
        state.apply_account_update(&update).unwrap();

        let val = state.get_storage_slot(&addr, slot_key).unwrap();
        assert_eq!(val, U256::from(777u64));
    }

    // 9. Writing zero deletes storage slot.
    #[test]
    fn test_apply_account_update_storage_delete() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x13);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(0, 0));
        state.apply_genesis(&accounts).unwrap();

        let slot_key = H256(U256::from(3u64).to_big_endian());

        // Write a value first.
        let mut update = AccountUpdate::new(addr);
        update.added_storage.insert(slot_key, U256::from(123u64));
        state.apply_account_update(&update).unwrap();

        assert!(state.get_storage_slot(&addr, slot_key).is_some());

        // Write zero → should delete.
        let mut update2 = AccountUpdate::new(addr);
        update2.added_storage.insert(slot_key, U256::zero());
        state.apply_account_update(&update2).unwrap();

        assert!(state.get_storage_slot(&addr, slot_key).is_none());
        // storage_root should be EMPTY_TRIE_HASH after deletion.
        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.storage_root, *EMPTY_TRIE_HASH);
    }

    // 10. removed=true clears all account data.
    #[test]
    fn test_apply_account_update_remove_account() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x14);

        let bytecode = Bytes::from(vec![0x00u8; 31]);
        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_contract(100, bytecode));
        state.apply_genesis(&accounts).unwrap();

        assert!(state.get_account_state(&addr).is_some());

        let update = AccountUpdate::removed(addr);
        state.apply_account_update(&update).unwrap();

        assert!(state.get_account_state(&addr).is_none());
    }

    // 11. removed_storage=true clears storage but keeps account.
    #[test]
    fn test_apply_account_update_removed_storage() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x15);

        let mut storage = BTreeMap::new();
        storage.insert(U256::from(0u64), U256::from(10u64));
        storage.insert(U256::from(1u64), U256::from(20u64));

        let genesis_account = GenesisAccount {
            code: Bytes::new(),
            storage,
            balance: U256::from(500u64),
            nonce: 3,
        };

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, genesis_account);
        state.apply_genesis(&accounts).unwrap();

        // Storage exists.
        assert!(
            state
                .get_storage_slot(&addr, H256(U256::from(0u64).to_big_endian()))
                .is_some()
        );

        // Apply update with removed_storage=true but keep the account.
        let mut update = AccountUpdate::new(addr);
        update.removed_storage = true;
        update.info = Some(AccountInfo {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: U256::from(500u64),
            nonce: 3,
        });
        state.apply_account_update(&update).unwrap();

        // Account still exists.
        assert!(state.get_account_state(&addr).is_some());

        // Storage is cleared.
        assert!(
            state
                .get_storage_slot(&addr, H256(U256::from(0u64).to_big_endian()))
                .is_none()
        );
        assert!(
            state
                .get_storage_slot(&addr, H256(U256::from(1u64).to_big_endian()))
                .is_none()
        );
    }

    // 12. storage_root synthesis: no storage → EMPTY_TRIE_HASH, has storage → non-empty.
    #[test]
    fn test_storage_root_synthesis() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x20);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(100, 0));
        state.apply_genesis(&accounts).unwrap();

        // No storage yet.
        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.storage_root, *EMPTY_TRIE_HASH);

        // Add storage.
        let slot_key = H256(U256::from(0u64).to_big_endian());
        let mut update = AccountUpdate::new(addr);
        update.added_storage.insert(slot_key, U256::from(1u64));
        state.apply_account_update(&update).unwrap();

        let account_state2 = state.get_account_state(&addr).unwrap();
        assert_ne!(account_state2.storage_root, *EMPTY_TRIE_HASH);
    }

    // 13. State root changes after mutation.
    #[test]
    fn test_state_root_changes_on_mutation() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x30);

        let root_empty = state.state_root();

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(1, 0));
        state.apply_genesis(&accounts).unwrap();

        let root_after = state.state_root();
        assert_ne!(root_empty, root_after);
    }

    // 14. Same operations produce the same root (determinism).
    #[test]
    fn test_state_root_deterministic() {
        let addr1 = make_address(0x40);
        let addr2 = make_address(0x41);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr1, make_genesis_eoa(100, 1));
        accounts.insert(addr2, make_genesis_eoa(200, 2));

        let mut state1 = BinaryTrieState::new();
        state1.apply_genesis(&accounts).unwrap();

        let mut state2 = BinaryTrieState::new();
        state2.apply_genesis(&accounts).unwrap();

        assert_eq!(state1.state_root(), state2.state_root());
    }

    // Extra: get_account_code returns None for unknown hash.
    #[test]
    fn test_get_account_code_unknown_hash() {
        let mut state = BinaryTrieState::new();
        assert!(state.get_account_code(&H256::zero()).is_none());
    }

    // Extra: FxHashMap usage in AccountUpdate compiles fine with our apply.
    #[test]
    fn test_apply_account_update_empty_update() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x50);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(99, 0));
        state.apply_genesis(&accounts).unwrap();

        // Empty update (no info, no code, no storage changes) — should be a no-op.
        let update = AccountUpdate::new(addr);
        state.apply_account_update(&update).unwrap();

        let account_state = state.get_account_state(&addr).unwrap();
        assert_eq!(account_state.balance, U256::from(99u64));
    }

    // Code replacement: shrinking code must remove stale chunks.
    #[test]
    fn test_apply_account_update_code_replacement_shrink() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x60);

        // Genesis: deploy 62-byte code (2 chunks).
        let big_code = Bytes::from(vec![0x5Bu8; 62]);
        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_contract(100, big_code));
        state.apply_genesis(&accounts).unwrap();
        assert_eq!(state.get_code_size(&addr), 62);

        // Replace with 31-byte code (1 chunk).
        let small_code = Bytes::from(vec![0x00u8; 31]);
        let new_code = Code::from_bytecode(small_code.clone());
        let mut update = AccountUpdate::new(addr);
        update.info = Some(AccountInfo {
            code_hash: new_code.hash,
            balance: U256::from(100u64),
            nonce: 1,
        });
        update.code = Some(new_code.clone());
        state.apply_account_update(&update).unwrap();

        // Verify code_size is now 31.
        assert_eq!(state.get_code_size(&addr), 31);

        // Verify old chunk 1 is gone (key for chunk_id=1 should return None).
        let chunk1_key = crate::key_mapping::get_tree_key_for_code_chunk(&addr, 1);
        assert!(
            state.trie.get(chunk1_key).is_none(),
            "stale chunk 1 should have been removed"
        );

        // Verify new code is retrievable.
        let retrieved = state.get_account_code(&new_code.hash).unwrap();
        assert_eq!(retrieved, small_code);
    }

    // Two accounts sharing bytecode: removing one doesn't break the other.
    #[test]
    fn test_shared_code_removal() {
        let mut state = BinaryTrieState::new();
        let addr1 = make_address(0x70);
        let addr2 = make_address(0x71);
        let bytecode = Bytes::from(vec![0x60u8, 0x00, 0x56]);
        let code_hash = keccak(bytecode.as_ref());

        let mut accounts = BTreeMap::new();
        accounts.insert(addr1, make_genesis_contract(100, bytecode.clone()));
        accounts.insert(addr2, make_genesis_contract(200, bytecode.clone()));
        state.apply_genesis(&accounts).unwrap();

        // Remove first account.
        let update = AccountUpdate::removed(addr1);
        state.apply_account_update(&update).unwrap();

        // Second account's code is still accessible.
        assert!(state.get_account_code(&code_hash).is_some());
        let acct2 = state.get_account_state(&addr2).unwrap();
        assert_eq!(acct2.code_hash, code_hash);
    }

    // removed_storage + new storage in same update: old gone, new present.
    #[test]
    fn test_removed_storage_then_new_storage() {
        let mut state = BinaryTrieState::new();
        let addr = make_address(0x80);

        let mut storage = BTreeMap::new();
        storage.insert(U256::from(0u64), U256::from(111u64));
        let genesis_account = GenesisAccount {
            code: Bytes::new(),
            storage,
            balance: U256::from(50u64),
            nonce: 0,
        };
        let mut accounts = BTreeMap::new();
        accounts.insert(addr, genesis_account);
        state.apply_genesis(&accounts).unwrap();

        // removed_storage + write new slot in same update.
        let new_slot = H256(U256::from(99u64).to_big_endian());
        let mut update = AccountUpdate::new(addr);
        update.removed_storage = true;
        update.info = Some(AccountInfo {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: U256::from(50u64),
            nonce: 1,
        });
        update.added_storage.insert(new_slot, U256::from(222u64));
        state.apply_account_update(&update).unwrap();

        // Old slot gone.
        assert!(
            state
                .get_storage_slot(&addr, H256(U256::from(0u64).to_big_endian()))
                .is_none()
        );
        // New slot present.
        assert_eq!(
            state.get_storage_slot(&addr, new_slot).unwrap(),
            U256::from(222u64)
        );
    }

    // Concurrent reads via &self.
    #[test]
    fn test_concurrent_state_reads() {
        use std::sync::{Arc, RwLock};

        let mut state = BinaryTrieState::new();
        let addr = make_address(0xCC);

        let mut accounts = BTreeMap::new();
        accounts.insert(addr, make_genesis_eoa(12345, 7));
        state.apply_genesis(&accounts).unwrap();

        let state = Arc::new(RwLock::new(state));

        let s1 = Arc::clone(&state);
        let s2 = Arc::clone(&state);

        let t1 = std::thread::spawn(move || {
            for _ in 0..500 {
                let s = s1.read().unwrap();
                let acct = s.get_account_state(&make_address(0xCC)).unwrap();
                assert_eq!(acct.balance, U256::from(12345u64));
            }
        });

        let t2 = std::thread::spawn(move || {
            for _ in 0..500 {
                let s = s2.read().unwrap();
                let acct = s.get_account_state(&make_address(0xCC)).unwrap();
                assert_eq!(acct.nonce, 7);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    }

    // ── Serialization round-trip tests ─────────────────────────────────

    #[cfg(feature = "rocksdb")]
    mod serde_tests {
        use super::*;
        use crate::state::{StateDiff, deserialize_diff_record, serialize_diff_record};
        use ethrex_common::types::AccountState;

        fn make_h256(n: u64) -> H256 {
            H256::from_low_u64_be(n)
        }

        fn make_addr(b: u8) -> Address {
            let mut a = [0u8; 20];
            a[19] = b;
            Address::from(a)
        }

        #[test]
        fn test_serde_empty_diff() {
            let parent = make_h256(1);
            let block_number = 100u64;
            let diff = StateDiff::default();

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (p2, bn2, d2) = deserialize_diff_record(&bytes).unwrap();

            assert_eq!(p2, parent);
            assert_eq!(bn2, block_number);
            assert!(d2.accounts.is_empty());
            assert!(d2.storage.is_empty());
            assert!(d2.code.is_empty());
            assert!(d2.storage_cleared.is_empty());
        }

        #[test]
        fn test_serde_with_accounts() {
            let parent = make_h256(42);
            let block_number = 999u64;
            let mut diff = StateDiff::default();

            let addr1 = make_addr(0x01);
            let addr2 = make_addr(0x02);

            // Present account
            diff.accounts.insert(
                addr1,
                Some(AccountState {
                    nonce: 7,
                    balance: U256::from(1_000_000u64),
                    storage_root: make_h256(0xAA),
                    code_hash: make_h256(0xBB),
                }),
            );
            // Deleted account
            diff.accounts.insert(addr2, None);

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (p2, bn2, d2) = deserialize_diff_record(&bytes).unwrap();

            assert_eq!(p2, parent);
            assert_eq!(bn2, block_number);

            let state1 = d2.accounts[&addr1].as_ref().unwrap();
            assert_eq!(state1.nonce, 7);
            assert_eq!(state1.balance, U256::from(1_000_000u64));
            assert_eq!(state1.storage_root, make_h256(0xAA));
            assert_eq!(state1.code_hash, make_h256(0xBB));

            assert!(d2.accounts[&addr2].is_none());
        }

        #[test]
        fn test_serde_with_storage() {
            let parent = make_h256(0);
            let block_number = 1u64;
            let mut diff = StateDiff::default();

            let addr = make_addr(0x10);
            let key1 = make_h256(1);
            let key2 = make_h256(2);

            diff.storage
                .insert((addr, key1), Some(U256::from(777u64)));
            diff.storage.insert((addr, key2), None); // deleted

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (_, _, d2) = deserialize_diff_record(&bytes).unwrap();

            assert_eq!(d2.storage[&(addr, key1)], Some(U256::from(777u64)));
            assert_eq!(d2.storage[&(addr, key2)], None);
        }

        #[test]
        fn test_serde_with_code() {
            let parent = make_h256(10);
            let block_number = 50u64;
            let mut diff = StateDiff::default();

            let code_bytes = Bytes::from(vec![0x60u8, 0x00, 0x56]);
            let code_hash = make_h256(0xCC);
            diff.code.insert(code_hash, code_bytes.clone());

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (_, _, d2) = deserialize_diff_record(&bytes).unwrap();

            assert_eq!(d2.code[&code_hash], code_bytes);
        }

        #[test]
        fn test_serde_with_storage_cleared() {
            let parent = make_h256(5);
            let block_number = 200u64;
            let mut diff = StateDiff::default();

            let addr1 = make_addr(0x20);
            let addr2 = make_addr(0x21);
            diff.storage_cleared.insert(addr1);
            diff.storage_cleared.insert(addr2);

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (_, _, d2) = deserialize_diff_record(&bytes).unwrap();

            assert!(d2.storage_cleared.contains(&addr1));
            assert!(d2.storage_cleared.contains(&addr2));
            assert_eq!(d2.storage_cleared.len(), 2);
        }

        #[test]
        fn test_serde_all_fields() {
            let parent = make_h256(0xDEAD);
            let block_number = 12345u64;
            let mut diff = StateDiff::default();

            let addr = make_addr(0x30);
            let key = make_h256(0xFF);

            diff.accounts.insert(
                addr,
                Some(AccountState {
                    nonce: 1,
                    balance: U256::from(500u64),
                    storage_root: make_h256(1),
                    code_hash: make_h256(2),
                }),
            );
            diff.storage
                .insert((addr, key), Some(U256::from(999u64)));
            diff.code
                .insert(make_h256(2), Bytes::from(vec![0x00u8; 10]));
            diff.storage_cleared.insert(addr);

            let bytes = serialize_diff_record(&parent, block_number, &diff);
            let (p2, bn2, d2) = deserialize_diff_record(&bytes).unwrap();

            assert_eq!(p2, parent);
            assert_eq!(bn2, block_number);
            assert!(d2.accounts.contains_key(&addr));
            assert!(d2.storage.contains_key(&(addr, key)));
            assert!(d2.code.contains_key(&make_h256(2)));
            assert!(d2.storage_cleared.contains(&addr));
        }
    }

    // ── Witness generation and trie reconstruction tests ────────────────

    #[cfg(feature = "rocksdb")]
    mod witness_tests {
        use super::*;
        use std::collections::{HashMap, HashSet};

        fn make_address(b: u8) -> Address {
            let mut a = [0u8; 20];
            a[19] = b;
            Address::from(a)
        }

        fn tempdir() -> tempfile::TempDir {
            tempfile::tempdir().expect("failed to create tempdir")
        }

        #[test]
        fn test_generate_witness_basic() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xAA);
            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: Bytes::new(),
                    storage: BTreeMap::new(),
                    balance: U256::from(1000u64),
                    nonce: 5,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            state.state_root(); // merkelise

            let genesis_hash = H256::from_low_u64_be(0x1234);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);
            state.state_root(); // re-cache hashes after flush

            // Generate witness: account was accessed, no storage.
            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![]);
            let block_hash = H256::from_low_u64_be(0x5678);
            let witness = state
                .generate_witness(1, block_hash, &accessed, &HashSet::new(), vec![])
                .unwrap();

            assert_eq!(witness.block_number, 1);
            assert_eq!(witness.block_hash, block_hash);
            assert_ne!(witness.pre_state_root, [0u8; 32]);
            assert_eq!(witness.account_proofs.len(), 1);

            let entry = &witness.account_proofs[0];
            assert_eq!(entry.address, addr);
            assert_eq!(entry.balance, U256::from(1000u64));
            assert_eq!(entry.nonce, 5);
            assert_eq!(entry.code_hash, *EMPTY_KECCACK_HASH);
            assert!(!entry.basic_data_proof.siblings.is_empty());
        }

        #[test]
        fn test_generate_witness_with_storage() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xBB);
            let slot_key = U256::from(7u64);
            let slot_h256 = H256(slot_key.to_big_endian());
            let mut storage = BTreeMap::new();
            storage.insert(slot_key, U256::from(42u64));

            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: Bytes::new(),
                    storage,
                    balance: U256::from(100u64),
                    nonce: 0,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            state.state_root();

            let genesis_hash = H256::from_low_u64_be(0xAAAA);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);
            state.state_root(); // re-cache hashes after flush

            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![slot_h256]);
            let block_hash = H256::from_low_u64_be(0xBBBB);
            let witness = state
                .generate_witness(1, block_hash, &accessed, &HashSet::new(), vec![])
                .unwrap();

            assert_eq!(witness.storage_proofs.len(), 1);
            let sp = &witness.storage_proofs[0];
            assert_eq!(sp.address, addr);
            assert_eq!(sp.slot, slot_h256);
            assert_eq!(sp.value, U256::from(42u64));
            assert!(!sp.proof.siblings.is_empty());
        }

        #[test]
        fn test_generate_witness_with_code() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xCC);
            let bytecode = Bytes::from(vec![0x60u8, 0x00, 0x56]);
            let code_hash = keccak(bytecode.as_ref());

            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: bytecode.clone(),
                    storage: BTreeMap::new(),
                    balance: U256::zero(),
                    nonce: 1,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            state.state_root();

            let genesis_hash = H256::from_low_u64_be(0xCCCC);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);
            state.state_root(); // re-cache hashes after flush

            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![]);
            let mut accessed_codes = HashSet::new();
            accessed_codes.insert(code_hash);

            let block_hash = H256::from_low_u64_be(0xDDDD);
            let witness = state
                .generate_witness(1, block_hash, &accessed, &accessed_codes, vec![])
                .unwrap();

            assert_eq!(witness.codes.len(), 1);
            assert_eq!(witness.codes[0].code_hash, code_hash);
            assert_eq!(witness.codes[0].bytecode, bytecode.to_vec());

            // Account entry should have the code_hash.
            assert_eq!(witness.account_proofs[0].code_hash, code_hash);
        }

        #[test]
        fn test_reconstruct_at_block() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xDD);
            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: Bytes::new(),
                    storage: BTreeMap::new(),
                    balance: U256::from(100u64),
                    nonce: 0,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            let genesis_root = state.state_root();

            let genesis_hash = H256::from_low_u64_be(1);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);

            // Apply block 1: change balance to 200.
            let block1_hash = H256::from_low_u64_be(2);
            state.begin_block(block1_hash, genesis_hash, 1);
            let mut update = AccountUpdate::new(addr);
            update.info = Some(AccountInfo {
                code_hash: *EMPTY_KECCACK_HASH,
                balance: U256::from(200u64),
                nonce: 1,
            });
            state
                .apply_account_update_for_block(&update, block1_hash)
                .unwrap();
            state.persist_diff(block1_hash).unwrap();
            let post_root = state.state_root();

            // Roots should differ.
            assert_ne!(genesis_root, post_root);

            // Reconstruct at genesis (pre-state of block 1).
            let reconstructed = state.reconstruct_at_block(genesis_hash).unwrap();

            // The reconstructed trie should have the genesis balance.
            let acct = reconstructed.get_account_state(&addr).unwrap();
            assert_eq!(acct.balance, U256::from(100u64));
            assert_eq!(acct.nonce, 0);
        }

        #[test]
        fn test_reconstruct_and_generate_witness() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xEE);
            let slot_key = U256::from(10u64);
            let slot_h256 = H256(slot_key.to_big_endian());
            let mut storage = BTreeMap::new();
            storage.insert(slot_key, U256::from(50u64));

            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: Bytes::new(),
                    storage,
                    balance: U256::from(500u64),
                    nonce: 3,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            state.state_root();

            let genesis_hash = H256::from_low_u64_be(0x10);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);

            // Apply block 1: change balance and storage.
            let block1_hash = H256::from_low_u64_be(0x20);
            state.begin_block(block1_hash, genesis_hash, 1);
            let mut update = AccountUpdate::new(addr);
            update.info = Some(AccountInfo {
                code_hash: *EMPTY_KECCACK_HASH,
                balance: U256::from(999u64),
                nonce: 4,
            });
            update
                .added_storage
                .insert(slot_h256, U256::from(100u64));
            state
                .apply_account_update_for_block(&update, block1_hash)
                .unwrap();
            state.persist_diff(block1_hash).unwrap();
            state.state_root();

            // Reconstruct pre-state of block 1 and generate witness.
            let mut reconstructed = state.reconstruct_at_block(genesis_hash).unwrap();
            reconstructed.state_root(); // merkelise

            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![slot_h256]);
            let witness = reconstructed
                .generate_witness(1, block1_hash, &accessed, &HashSet::new(), vec![])
                .unwrap();

            // Pre-state values should be genesis values, NOT block 1 values.
            let entry = &witness.account_proofs[0];
            assert_eq!(entry.balance, U256::from(500u64));
            assert_eq!(entry.nonce, 3);

            let sp = &witness.storage_proofs[0];
            assert_eq!(sp.value, U256::from(50u64));
        }

        #[test]
        fn test_reconstruct_multiple_blocks() {
            let dir = tempdir();
            let mut state = BinaryTrieState::open(dir.path()).unwrap();

            let addr = make_address(0xFF);
            let mut accounts = BTreeMap::new();
            accounts.insert(
                addr,
                GenesisAccount {
                    code: Bytes::new(),
                    storage: BTreeMap::new(),
                    balance: U256::from(10u64),
                    nonce: 0,
                },
            );
            state.apply_genesis(&accounts).unwrap();
            state.state_root();

            let genesis_hash = H256::from_low_u64_be(0x01);
            state.flush(0, genesis_hash).unwrap();
            state.set_diff_base(genesis_hash, 0);

            // Apply 5 blocks, each incrementing balance by 10.
            let mut parent = genesis_hash;
            for i in 1..=5u64 {
                let block_hash = H256::from_low_u64_be(0x01 + i);
                state.begin_block(block_hash, parent, i);
                let mut update = AccountUpdate::new(addr);
                update.info = Some(AccountInfo {
                    code_hash: *EMPTY_KECCACK_HASH,
                    balance: U256::from(10 + i * 10),
                    nonce: i,
                });
                state
                    .apply_account_update_for_block(&update, block_hash)
                    .unwrap();
                state.persist_diff(block_hash).unwrap();
                state.state_root();
                parent = block_hash;
            }

            // Reconstruct at block 3's parent (block 2) -> pre-state of block 3.
            let block2_hash = H256::from_low_u64_be(0x03); // block 2
            let reconstructed = state.reconstruct_at_block(block2_hash).unwrap();
            let acct = reconstructed.get_account_state(&addr).unwrap();
            // After block 2: balance = 10 + 2*10 = 30, nonce = 2
            assert_eq!(acct.balance, U256::from(30u64));
            assert_eq!(acct.nonce, 2);
        }
    }
}
