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

// Meta keys stored in BINARY_TRIE_NODES CF (alongside node IDs).
// The 0xFF prefix ensures they don't collide with 8-byte u64 node keys.
#[cfg(feature = "rocksdb")]
const META_BLOCK_KEY: &[u8] = &[0xFF, b'B'];
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

    /// Shared RocksDB handle (present only when opened with `open_with_db()`).
    #[cfg(feature = "rocksdb")]
    db: Option<Arc<rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>>>,

    /// Column family name for code (code_hash -> bytecode).
    #[cfg(feature = "rocksdb")]
    code_cf: &'static str,

    /// Column family name for storage key tracking (address -> packed H256 list).
    #[cfg(feature = "rocksdb")]
    storage_keys_cf: &'static str,

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
            code_cf: "",
            #[cfg(feature = "rocksdb")]
            storage_keys_cf: "",
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

    /// Open a persistent `BinaryTrieState` using a shared RocksDB instance.
    ///
    /// All reads/writes use the named column families `nodes_cf`, `code_cf`,
    /// and `storage_keys_cf`, which must already exist in `db`.
    ///
    /// If the database already contains data (the trie root is present),
    /// the trie nodes, code store, and storage keys are loaded from it.
    /// If the database is new/empty, an empty state is returned — the
    /// caller is responsible for applying genesis.
    #[cfg(feature = "rocksdb")]
    pub fn open_with_db(
        db: Arc<rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>>,
        nodes_cf: &'static str,
        code_cf: &'static str,
        storage_keys_cf: &'static str,
    ) -> Result<Self, BinaryTrieError> {
        let store = NodeStore::open(Arc::clone(&db), nodes_cf)?;
        let root = store.load_root();
        let trie = BinaryTrie { store, root };

        let (code_store, storage_keys) = if root.is_some() {
            // Existing database — load side structures.
            let code_store = load_code_store(&db, code_cf)?;
            let storage_keys = load_storage_keys(&db, storage_keys_cf)?;
            (code_store, storage_keys)
        } else {
            (FxHashMap::default(), FxHashMap::default())
        };

        // The flushed root on disk IS the base for diff layers.
        let base_root = root;

        // Restore the diff tree base_hash from disk so that disk-backed
        // backward walks know where to stop and fall through to the base trie.
        // Use a scope to ensure the CF handle is dropped before `db` is moved below.
        let base_hash = {
            let nodes_cf_handle = db
                .cf_handle(nodes_cf)
                .ok_or_else(|| BinaryTrieError::StoreError(format!("CF '{nodes_cf}' not found")))?;
            db.get_cf(&nodes_cf_handle, META_BASE_HASH_KEY)
                .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
                .and_then(|bytes| {
                    if bytes.len() >= 32 {
                        Some(H256::from_slice(&bytes[..32]))
                    } else {
                        None
                    }
                })
                .unwrap_or(H256::zero())
            // nodes_cf_handle dropped here
        };

        let mut diff_tree = DiffTree::new();
        diff_tree.base_hash = base_hash;

        Ok(Self {
            trie,
            diff_tree,
            base_root,
            code_store: Mutex::new(code_store),
            storage_keys: Mutex::new(storage_keys),
            db: Some(db),
            code_cf,
            storage_keys_cf,
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
        let cf = db.cf_handle(self.trie.store.nodes_cf)?;
        let bytes = db.get_cf(&cf, META_BLOCK_KEY).ok()??;
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

        let nodes_cf = db.cf_handle(self.trie.store.nodes_cf).ok_or_else(|| {
            BinaryTrieError::StoreError(format!("CF '{}' not found", self.trie.store.nodes_cf))
        })?;
        let code_cf = db.cf_handle(self.code_cf).ok_or_else(|| {
            BinaryTrieError::StoreError(format!("CF '{}' not found", self.code_cf))
        })?;
        let storage_keys_cf = db.cf_handle(self.storage_keys_cf).ok_or_else(|| {
            BinaryTrieError::StoreError(format!("CF '{}' not found", self.storage_keys_cf))
        })?;

        let mut batch = rocksdb::WriteBatch::default();

        // 1. Flush trie nodes (dirty + freed nodes, root, next_id) into nodes_cf.
        self.trie.flush_to_batch(&mut batch, &nodes_cf);

        // 2. Write dirty code_store entries: raw code_hash (32 bytes) as key.
        {
            let code_store = self.code_store.lock().unwrap();
            for hash in &self.dirty_codes {
                if let Some(code) = code_store.get(hash) {
                    batch.put_cf(&code_cf, hash.as_bytes(), code.as_ref());
                } else {
                    // Code was removed — delete the entry.
                    batch.delete_cf(&code_cf, hash.as_bytes());
                }
            }
        }

        // 3. Write dirty storage_keys entries: raw address (20 bytes) as key.
        {
            let storage_keys = self.storage_keys.lock().unwrap();
            for addr in &self.dirty_storage_keys {
                if let Some(keys) = storage_keys.get(addr) {
                    let mut value = Vec::with_capacity(keys.len() * 32);
                    for k in keys {
                        value.extend_from_slice(k.as_bytes());
                    }
                    batch.put_cf(&storage_keys_cf, addr.as_bytes(), &value);
                } else {
                    // Account's storage was fully cleared — delete the entry.
                    batch.delete_cf(&storage_keys_cf, addr.as_bytes());
                }
            }
        }

        // 4. Write checkpoint block number and base block hash into nodes_cf (meta).
        batch.put_cf(&nodes_cf, META_BLOCK_KEY, block_number.to_le_bytes());
        batch.put_cf(&nodes_cf, META_BASE_HASH_KEY, block_hash.as_bytes());

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
            if let Some(cf) = db.cf_handle(self.code_cf) {
                if let Ok(Some(bytes)) = db.get_cf(&cf, code_hash.as_bytes()) {
                    let code = Bytes::copy_from_slice(&bytes);
                    self.code_store
                        .lock()
                        .unwrap()
                        .insert(*code_hash, code.clone());
                    return Some(code);
                }
            }
        }
        None
    }

    // ── Diff-layer-only reads (FKV fallback path) ─────────────────────

    /// Look up account in diff layers only, without falling through to the base trie.
    ///
    /// Returns:
    /// - `Some(Some(state))` — found in a diff layer.
    /// - `Some(None)` — explicitly deleted in a diff layer.
    /// - `None` — not in any diff layer; caller should fall through to FKV.
    pub fn diff_lookup_account(
        &self,
        address: &Address,
        block_hash: H256,
    ) -> Option<Option<AccountState>> {
        if block_hash.is_zero() {
            return None;
        }
        match self.diff_tree.get_account(address, block_hash) {
            DiffLookup::Found(state) => Some(Some(state)),
            DiffLookup::Deleted => Some(None),
            DiffLookup::NotModified | DiffLookup::NotInMemory => None,
        }
    }

    /// Look up storage in diff layers only, without falling through to the base trie.
    ///
    /// Returns:
    /// - `Some(Some(val))` — found in a diff layer.
    /// - `Some(None)` — explicitly zeroed/deleted in a diff layer.
    /// - `None` — not in any diff layer; caller should fall through to FKV.
    pub fn diff_lookup_storage(
        &self,
        address: &Address,
        key: H256,
        block_hash: H256,
    ) -> Option<Option<U256>> {
        if block_hash.is_zero() {
            return None;
        }
        match self.diff_tree.get_storage(address, key, block_hash) {
            DiffLookup::Found(val) => Some(Some(val)),
            DiffLookup::Deleted => Some(None),
            DiffLookup::NotModified | DiffLookup::NotInMemory => None,
        }
    }

    // ── Diff-layer-aware reads (for historical state) ─────────────────

    /// Read account state at a specific block (identified by block hash).
    ///
    /// `H256::zero()` is a special bypass used by tests;
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
            DiffLookup::NotInMemory => Ok(None),
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
            DiffLookup::NotInMemory => Ok(None),
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
            DiffLookup::NotInMemory => Ok(None),
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

    /// Set only the diff tree's base block number, leaving base_hash unchanged.
    /// Use this when reopening from disk: base_hash is already restored by
    /// `open_with_db` from `META_BASE_HASH_KEY`, so only the block number needs setting.
    pub fn set_diff_base_block(&mut self, block_number: u64) {
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
            get_tree_key_for_basic_data, get_tree_key_for_code_hash, get_tree_key_for_storage_slot,
            unpack_basic_data,
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
                let code_hash = ch_proof.value.map(H256).unwrap_or(*EMPTY_KECCACK_HASH);
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
        let Some(cf) = db.cf_handle(self.storage_keys_cf) else {
            return false;
        };
        if let Ok(Some(value)) = db.get_cf(&cf, address.as_bytes()) {
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

/// Load the code_store from a dedicated CF by iterating over all entries.
/// Keys are raw code_hash (32 bytes); values are raw bytecode.
#[cfg(feature = "rocksdb")]
fn load_code_store(
    db: &rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>,
    code_cf: &str,
) -> Result<FxHashMap<H256, Bytes>, BinaryTrieError> {
    let cf = db
        .cf_handle(code_cf)
        .ok_or_else(|| BinaryTrieError::StoreError(format!("CF '{code_cf}' not found")))?;
    let mut map = FxHashMap::default();
    let iter = db.full_iterator_cf(&cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item.map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        if key.len() < 32 {
            continue;
        }
        let hash = H256::from_slice(&key[..32]);
        map.insert(hash, Bytes::copy_from_slice(&value));
    }
    Ok(map)
}

/// Load storage_keys from a dedicated CF by iterating over all entries.
/// Keys are raw address (20 bytes); values are packed H256 storage keys.
#[cfg(feature = "rocksdb")]
fn load_storage_keys(
    db: &rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>,
    storage_keys_cf: &str,
) -> Result<FxHashMap<Address, FxHashSet<H256>>, BinaryTrieError> {
    let cf = db
        .cf_handle(storage_keys_cf)
        .ok_or_else(|| BinaryTrieError::StoreError(format!("CF '{storage_keys_cf}' not found")))?;
    let mut map: FxHashMap<Address, FxHashSet<H256>> = FxHashMap::default();
    let iter = db.full_iterator_cf(&cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item.map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        if key.len() < 20 {
            continue;
        }
        let addr = Address::from_slice(&key[..20]);
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
    use ethrex_crypto::NativeCrypto;

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
        let code = Code::from_bytecode(bytecode.clone(), &NativeCrypto);
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
        let new_code = Code::from_bytecode(small_code.clone(), &NativeCrypto);
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

    // ── Witness generation tests ─────────────────────────────────────────

    #[cfg(feature = "rocksdb")]
    mod witness_tests {
        use super::*;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        const NODES_CF: &str = "binary_trie_nodes";
        const CODE_CF: &str = "binary_trie_code";
        const STORAGE_KEYS_CF: &str = "binary_trie_storage_keys";

        fn make_address(b: u8) -> Address {
            let mut a = [0u8; 20];
            a[19] = b;
            Address::from(a)
        }

        fn tempdir() -> tempfile::TempDir {
            tempfile::tempdir().expect("failed to create tempdir")
        }

        /// Open a test RocksDB with the 3 binary trie column families.
        fn open_test_db(
            dir: &tempfile::TempDir,
        ) -> Arc<rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>> {
            let mut opts = rocksdb::Options::default();
            opts.create_if_missing(true);
            opts.create_missing_column_families(true);
            let cfs = vec![
                rocksdb::ColumnFamilyDescriptor::new(NODES_CF, opts.clone()),
                rocksdb::ColumnFamilyDescriptor::new(CODE_CF, opts.clone()),
                rocksdb::ColumnFamilyDescriptor::new(STORAGE_KEYS_CF, opts.clone()),
            ];
            Arc::new(
                rocksdb::DBWithThreadMode::open_cf_descriptors(&opts, dir.path(), cfs)
                    .expect("failed to open test RocksDB"),
            )
        }

        fn open_test_state(dir: &tempfile::TempDir) -> BinaryTrieState {
            let db = open_test_db(dir);
            BinaryTrieState::open_with_db(db, NODES_CF, CODE_CF, STORAGE_KEYS_CF).unwrap()
        }

        #[test]
        fn test_generate_witness_basic() {
            let dir = tempdir();
            let mut state = open_test_state(&dir);

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
            let mut state = open_test_state(&dir);

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
            let mut state = open_test_state(&dir);

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
    }
}
