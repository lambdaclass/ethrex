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
};

// Key prefixes for code_store and storage_keys in RocksDB.
// 0x01 is used by NodeStore for trie nodes; 0xFF for trie metadata.
#[cfg(feature = "rocksdb")]
const CODE_PREFIX: u8 = 0x02;
#[cfg(feature = "rocksdb")]
const STORAGE_KEYS_PREFIX: u8 = 0x03;
#[cfg(feature = "rocksdb")]
const META_BLOCK_KEY: &[u8] = &[0xFF, b'B'];

pub struct BinaryTrieState {
    /// The underlying binary trie holding all state leaves.
    trie: BinaryTrie,

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

        Ok(Self {
            trie,
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
    pub fn flush(&mut self, block_number: u64) -> Result<(), BinaryTrieError> {
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

        // 4. Write checkpoint block number.
        batch.put(META_BLOCK_KEY, block_number.to_le_bytes());

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

        Ok(())
    }

    /// Flush to disk if the block threshold has been reached.
    ///
    /// Returns `true` if a flush was performed, `false` otherwise.
    /// Call this after each block's `apply_account_update` calls.
    #[cfg(feature = "rocksdb")]
    pub fn flush_if_needed(&mut self, block_number: u64) -> Result<bool, BinaryTrieError> {
        self.blocks_since_flush += 1;
        if self.blocks_since_flush >= self.flush_threshold {
            self.flush(block_number)?;
            Ok(true)
        } else {
            Ok(false)
        }
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

    /// Apply a single AccountUpdate to the trie.
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
}
