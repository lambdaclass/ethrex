use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use rustc_hash::{FxHashMap, FxHashSet};

use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountInfo, AccountState, AccountUpdate, Code, GenesisAccount},
    utils::keccak,
};

use crate::{
    BinaryTrie,
    db::{TrieBackend, WriteOp},
    error::BinaryTrieError,
    key_mapping::{
        BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY, chunkify_code, get_stem_for_base,
        get_tree_key_for_basic_data, get_tree_key_for_code_chunk, get_tree_key_for_code_hash,
        get_tree_key_for_storage_slot, pack_basic_data, tree_key_from_stem, unpack_basic_data,
    },
    merkle::merkelize,
    node_store::NodeStore,
};

// Meta keys stored in BINARY_TRIE_NODES table (alongside node IDs).
// The 0xFF prefix ensures they don't collide with 8-byte u64 node keys.
const META_BLOCK_KEY: &[u8] = &[0xFF, b'B'];
const META_BASE_HASH_KEY: &[u8] = &[0xFF, b'H'];

impl std::fmt::Debug for BinaryTrieState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryTrieState").finish_non_exhaustive()
    }
}

pub struct BinaryTrieState {
    /// The underlying binary trie holding all state leaves.
    trie: BinaryTrie,

    /// Leaf diffs accumulated during the current block's `apply_account_update` calls.
    /// Drained via `take_block_diffs()` after each block's merkleization.
    current_block_diffs: Vec<([u8; 32], Option<[u8; 32]>)>,

    /// Binary trie state root from the previous block (before the current block's updates).
    /// Used as the `parent` when inserting a layer into the layer cache.
    prev_state_root: [u8; 32],

    /// Tracks which storage keys each account has written.
    /// Needed for `removed_storage` (SELFDESTRUCT) since the binary trie
    /// has no prefix-enumeration — we can't discover all storage keys
    /// for an address without this side structure.
    ///
    /// Wrapped in a RwLock so `has_storage_keys` can populate the cache with
    /// `&self` (concurrent reads from executor threads) while allowing
    /// multiple simultaneous readers without contention.
    storage_keys: RwLock<FxHashMap<Address, FxHashSet<H256>>>,

    /// Persistence backend (None for pure in-memory operation).
    backend: Option<Arc<dyn TrieBackend>>,

    /// Table name for storage key tracking (address -> packed H256 list).
    storage_keys_table: &'static str,

    /// Addresses whose storage_keys entry has changed since the last `flush()`.
    dirty_storage_keys: FxHashSet<Address>,

    /// Number of blocks applied since the last flush.
    blocks_since_flush: u64,

    /// Flush to disk when `blocks_since_flush` reaches this threshold.
    /// Default 128, matching MPT's `DB_COMMIT_THRESHOLD`.
    flush_threshold: u64,

    /// Block number of the last successful flush.
    last_flushed_block: u64,
}

impl BinaryTrieState {
    pub fn new() -> Self {
        Self {
            trie: BinaryTrie::new(),
            current_block_diffs: Vec::new(),
            prev_state_root: [0u8; 32],
            storage_keys: RwLock::new(FxHashMap::default()),
            backend: None,
            storage_keys_table: "",
            dirty_storage_keys: FxHashSet::default(),
            blocks_since_flush: 0,
            flush_threshold: 128,
            last_flushed_block: 0,
        }
    }

    /// Check if state is available for the given block.
    pub fn has_state_for_block(&self, _block_hash: H256, block_number: u64) -> bool {
        block_number <= self.last_flushed_block
    }

    /// Open a persistent `BinaryTrieState` using a `TrieBackend`.
    ///
    /// All reads/writes use the named tables `nodes_table` and
    /// `storage_keys_table`, which must already exist in the backend.
    ///
    /// If the database already contains data (the trie root is present),
    /// the trie nodes and storage keys are loaded from it.
    /// If the database is new/empty, an empty state is returned — the
    /// caller is responsible for applying genesis.
    pub fn open(
        backend: Arc<dyn TrieBackend>,
        nodes_table: &'static str,
        storage_keys_table: &'static str,
    ) -> Result<Self, BinaryTrieError> {
        let store = NodeStore::open(Arc::clone(&backend), nodes_table)?;
        let root = store.load_root();
        let trie = BinaryTrie { store, root };

        let storage_keys = if root.is_some() {
            load_storage_keys(backend.as_ref(), storage_keys_table)?
        } else {
            FxHashMap::default()
        };

        Ok(Self {
            trie,
            current_block_diffs: Vec::new(),
            prev_state_root: [0u8; 32],
            storage_keys: RwLock::new(storage_keys),
            backend: Some(backend),
            storage_keys_table,
            dirty_storage_keys: FxHashSet::default(),
            blocks_since_flush: 0,
            flush_threshold: 128,
            last_flushed_block: 0,
        })
    }

    /// Returns `true` if the trie contains any data (i.e. a root node exists).
    ///
    /// Used to decide whether genesis needs to be applied after `open()`.
    pub fn has_data(&self) -> bool {
        self.trie.root.is_some()
    }

    /// Returns the last block number recorded by `flush()`, if any.
    pub fn checkpoint_block(&self) -> Option<u64> {
        let backend = self.backend.as_ref()?;
        let bytes = backend
            .get(self.trie.store.nodes_table, META_BLOCK_KEY)
            .ok()??;
        if bytes.len() < 8 {
            return None;
        }
        Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
    }

    /// Persist the current state and record `block_number` as the checkpoint.
    ///
    /// All dirty trie nodes and storage_keys entries are written
    /// atomically. On success the dirty sets are cleared.
    pub fn flush(&mut self, block_number: u64, block_hash: H256) -> Result<(), BinaryTrieError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| BinaryTrieError::StoreError("no backend configured".into()))?
            .clone();
        let ops = self.prepare_flush(block_number, block_hash)?;
        backend.write_batch(ops)
    }

    /// Build a `Vec<WriteOp>` with all dirty trie nodes and storage_keys,
    /// then rotate generations and clear dirty state. The returned ops
    /// can be written to the backend asynchronously.
    ///
    /// After this call, the in-memory trie state is ready for the next block
    /// (dirty nodes moved to warm, caches stripped). The caller MUST write
    /// the ops to disk eventually for crash consistency.
    pub fn prepare_flush(
        &mut self,
        block_number: u64,
        block_hash: H256,
    ) -> Result<Vec<WriteOp>, BinaryTrieError> {
        if self.backend.is_none() {
            return Err(BinaryTrieError::StoreError("no backend configured".into()));
        }

        let nodes_table = self.trie.store.nodes_table;
        let storage_keys_table = self.storage_keys_table;

        // 1. Collect trie node ops (dirty + freed nodes, root, next_id).
        let mut ops = self.trie.collect_flush_ops();

        // 2. Write dirty storage_keys entries.
        {
            let storage_keys = self.storage_keys.read().unwrap();
            for addr in &self.dirty_storage_keys {
                if let Some(keys) = storage_keys.get(addr) {
                    let mut value = Vec::with_capacity(keys.len() * 32);
                    for k in keys {
                        value.extend_from_slice(k.as_bytes());
                    }
                    ops.push(WriteOp::Put {
                        table: storage_keys_table,
                        key: Box::from(addr.as_bytes()),
                        value,
                    });
                } else {
                    ops.push(WriteOp::Delete {
                        table: storage_keys_table,
                        key: Box::from(addr.as_bytes()),
                    });
                }
            }
        }

        // 3. Write checkpoint metadata.
        ops.push(WriteOp::Put {
            table: nodes_table,
            key: Box::from(META_BLOCK_KEY),
            value: block_number.to_le_bytes().to_vec(),
        });
        ops.push(WriteOp::Put {
            table: nodes_table,
            key: Box::from(META_BASE_HASH_KEY),
            value: block_hash.as_bytes().to_vec(),
        });

        // 4. In-memory rotation: clear dirty tracking, keep entries in cache.
        // Entries remain in storage_keys for fast reads; the next reload_from_checkpoint
        // will repopulate from disk if needed. Only the dirty set is cleared so the
        // next flush doesn't re-write unchanged entries.
        self.dirty_storage_keys.clear();
        self.blocks_since_flush = 0;
        self.last_flushed_block = block_number;

        Ok(ops)
    }

    /// Check if the flush threshold has been reached.
    /// Increments the block counter by `block_count` (1 for pipeline, batch size for batch).
    pub fn tick_and_check_flush(&mut self, block_count: u64) -> bool {
        self.blocks_since_flush += block_count;
        self.blocks_since_flush >= self.flush_threshold
    }

    /// Returns `true` if a flush was performed, `false` otherwise.
    /// Call this after each block's `apply_account_update` calls.
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

    /// Set the flush threshold (number of blocks between disk commits).
    pub fn set_flush_threshold(&mut self, threshold: u64) {
        self.flush_threshold = threshold;
    }

    /// Set the last flushed block number (used when resuming from a checkpoint).
    pub fn set_last_flushed_block(&mut self, block_number: u64) {
        self.last_flushed_block = block_number;
    }

    /// Returns the last flushed block number.
    pub fn last_flushed_block(&self) -> u64 {
        self.last_flushed_block
    }

    /// Returns the flush threshold (number of blocks between disk commits).
    pub fn flush_threshold(&self) -> u64 {
        self.flush_threshold
    }

    /// Reload the trie and storage keys from the last disk checkpoint,
    /// discarding all in-memory mutations since the last flush.
    ///
    /// Returns the checkpoint block number, or 0 if no checkpoint exists.
    /// Used during reorg recovery to restore the trie to a known-good state
    /// before re-executing blocks on the new fork.
    pub fn reload_from_checkpoint(&mut self) -> Result<u64, BinaryTrieError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| BinaryTrieError::StoreError("no backend configured".into()))?
            .clone();

        let store = NodeStore::open(Arc::clone(&backend), self.trie.store.nodes_table)?;
        let root = store.load_root();
        self.trie = BinaryTrie { store, root };

        let storage_keys = if root.is_some() {
            load_storage_keys(backend.as_ref(), self.storage_keys_table)?
        } else {
            FxHashMap::default()
        };
        *self.storage_keys.write().unwrap() = storage_keys;
        self.dirty_storage_keys.clear();
        self.blocks_since_flush = 0;
        self.current_block_diffs.clear();
        self.prev_state_root = [0u8; 32];

        let checkpoint = self.checkpoint_block().unwrap_or(0);
        self.last_flushed_block = checkpoint;

        Ok(checkpoint)
    }

    /// Compute the binary trie state root via merkelization.
    pub fn state_root(&mut self) -> [u8; 32] {
        merkelize(&mut self.trie)
    }

    /// Drain and return the leaf diffs accumulated during the current block,
    /// along with the parent root (the state root before this block's updates).
    ///
    /// Also advances `prev_state_root` to `new_root` so the next block's
    /// diffs will have the correct parent.
    ///
    /// Call this after `state_root()` returns `new_root`.
    pub fn take_block_diffs(
        &mut self,
        new_root: [u8; 32],
    ) -> ([u8; 32], Vec<([u8; 32], Option<[u8; 32]>)>) {
        let parent = self.prev_state_root;
        self.prev_state_root = new_root;
        (parent, std::mem::take(&mut self.current_block_diffs))
    }

    /// Record a trie insert into the current block's diffs.
    fn record_insert(&mut self, key: [u8; 32], value: [u8; 32]) {
        self.current_block_diffs.push((key, Some(value)));
    }

    /// Record a trie remove into the current block's diffs.
    fn record_remove(&mut self, key: [u8; 32]) {
        self.current_block_diffs.push((key, None));
    }

    /// Read a raw leaf from the binary trie by its 32-byte tree key.
    ///
    /// Returns `Some(value)` if the leaf exists, `None` otherwise.
    /// Used by the storage layer's unified read path (`BinaryTrieWrapper`).
    pub fn trie_get(&self, tree_key: [u8; 32]) -> Option<[u8; 32]> {
        self.trie.get_shared(tree_key)
    }

    /// Insert a raw leaf into the binary trie by its 32-byte tree key.
    ///
    /// Used by bulk migration tools (e.g. migrate subcommand) that compute
    /// keys externally and insert directly, bypassing the higher-level
    /// account/storage update path.
    pub fn trie_insert(
        &mut self,
        tree_key: [u8; 32],
        value: [u8; 32],
    ) -> Result<(), BinaryTrieError> {
        self.trie.insert(tree_key, value)
    }

    /// Read account state from the binary trie.
    ///
    /// Returns None if the account doesn't exist (no basic_data leaf).
    /// The `storage_root` field is synthesized:
    ///   - EMPTY_TRIE_HASH if the account has no tracked storage keys
    ///   - A dummy non-empty hash (H256::from_low_u64_be(1)) otherwise
    pub fn get_account_state(&self, address: &Address) -> Option<AccountState> {
        let basic_data_key = get_tree_key_for_basic_data(address);
        let basic_data = self.trie.get_shared(basic_data_key)?;

        let (_version, _code_size, nonce, balance) = unpack_basic_data(&basic_data);

        let code_hash_key = get_tree_key_for_code_hash(address);
        let code_hash = self
            .trie
            .get_shared(code_hash_key)
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
        self.trie
            .get_shared(tree_key)
            .map(|v| U256::from_big_endian(&v))
    }

    /// Get code size from basic_data. Returns 0 if account doesn't exist.
    pub fn get_code_size(&self, address: &Address) -> u32 {
        let basic_data_key = get_tree_key_for_basic_data(address);
        match self.trie.get_shared(basic_data_key) {
            Some(data) => {
                let (_version, code_size, _nonce, _balance) = unpack_basic_data(&data);
                code_size
            }
            None => 0,
        }
    }

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

        // Pre-compute the stem once for this address (tree_index=0).
        // Reused by write_code, write_account_info, and the code_hash fallback.
        let base_stem = get_stem_for_base(address);

        // Write code BEFORE account info — write_code reads old code_size from
        // basic_data to know how many old chunks to evict. write_account_info
        // overwrites basic_data with the new code_size, so it must come after.
        if let Some(ref code) = update.code {
            self.write_code(address, code, &base_stem)?;

            // If code changed but info wasn't provided (defensive — LEVM always
            // sends both together), update the code_hash leaf directly so the trie
            // stays consistent.
            if update.info.is_none() {
                let code_hash_key = tree_key_from_stem(&base_stem, CODE_HASH_LEAF_KEY);
                self.trie.insert(code_hash_key, code.hash.0)?;
                self.record_insert(code_hash_key, code.hash.0);
            }
        }

        // Apply account info changes (writes basic_data + code_hash).
        if let Some(ref info) = update.info {
            self.write_account_info(info, update.code.as_ref(), &base_stem)?;
        }

        // Apply storage changes: trie mutations first, then update storage_keys.
        if !update.added_storage.is_empty() {
            let mut keys_to_add = Vec::new();
            let mut keys_to_remove = Vec::new();

            for (key, value) in &update.added_storage {
                let storage_key = U256::from_big_endian(key.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(address, storage_key);

                if value.is_zero() {
                    self.trie.remove(tree_key)?;
                    self.record_remove(tree_key);
                    keys_to_remove.push(*key);
                } else {
                    let value_bytes = value.to_big_endian();
                    self.trie.insert(tree_key, value_bytes)?;
                    self.record_insert(tree_key, value_bytes);
                    keys_to_add.push(*key);
                }
            }

            // Batch-update storage_keys with a single write-lock acquisition.
            let mut storage_keys = self.storage_keys.write().unwrap();
            for key in keys_to_remove {
                if let Some(keys) = storage_keys.get_mut(address) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        storage_keys.remove(address);
                    }
                }
            }
            if !keys_to_add.is_empty() {
                let entry = storage_keys.entry(*address).or_default();
                for key in keys_to_add {
                    entry.insert(key);
                }
            }
            drop(storage_keys);
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

            // Write code chunks for merkleization.
            if !genesis.code.is_empty() {
                let chunks = chunkify_code(&genesis.code);
                for (i, chunk) in chunks.iter().enumerate() {
                    self.trie
                        .insert(get_tree_key_for_code_chunk(address, i as u64), *chunk)?;
                }
            }

            // Write storage slots.
            if !genesis.storage.is_empty() {
                let mut storage_keys = self.storage_keys.write().unwrap();
                for (slot, value) in &genesis.storage {
                    if !value.is_zero() {
                        let tree_key = get_tree_key_for_storage_slot(address, *slot);
                        self.trie.insert(tree_key, value.to_big_endian())?;

                        let key_h256 = H256(slot.to_big_endian());
                        storage_keys.entry(*address).or_default().insert(key_h256);
                        self.dirty_storage_keys.insert(*address);
                    }
                }
            }
        }
        Ok(())
    }

    /// Return sizes of the main in-memory collections for diagnostics.
    ///
    /// Returns `(clean_cache, warm, dirty_nodes, freed, storage_keys_accounts)`.
    pub fn memory_stats(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.trie.store.clean_cache_len(),
            self.trie.store.warm_len(),
            self.trie.store.dirty_len(),
            self.trie.store.freed_len(),
            self.storage_keys.read().unwrap().len(),
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
    /// - `codes`: map of code_hash -> bytecode (from the Store's ACCOUNT_CODES table)
    /// - `block_headers`: RLP-encoded headers needed for BLOCKHASH opcode
    pub fn generate_witness(
        &self,
        block_number: u64,
        block_hash: H256,
        accessed_accounts: &std::collections::HashMap<Address, Vec<H256>>,
        accessed_codes: &std::collections::HashSet<H256>,
        codes: &std::collections::HashMap<ethrex_common::H256, bytes::Bytes>,
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

        let mut witness_codes = Vec::new();
        for code_hash in accessed_codes {
            if let Some(bytecode) = codes.get(code_hash) {
                witness_codes.push(CodeWitnessEntry {
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
            codes: witness_codes,
            block_headers,
        })
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    /// Check whether `address` has any tracked storage keys.
    ///
    /// Checks in-memory cache first; on a miss, reloads from backend.
    pub fn has_storage_keys(&self, address: &Address) -> bool {
        {
            let cache = self.storage_keys.read().unwrap();
            if let Some(keys) = cache.get(address) {
                return !keys.is_empty();
            }
        }
        self.reload_storage_keys(address)
    }

    /// Reload storage_keys for a single address from the backend.
    /// Returns true if the address has non-empty storage keys.
    fn reload_storage_keys(&self, address: &Address) -> bool {
        let Some(backend) = self.backend.as_ref() else {
            return false;
        };
        if let Ok(Some(value)) = backend.get(self.storage_keys_table, address.as_bytes()) {
            if !value.is_empty() {
                let mut keys = FxHashSet::default();
                let mut offset = 0usize;
                while offset + 32 <= value.len() {
                    keys.insert(H256::from_slice(&value[offset..offset + 32]));
                    offset += 32;
                }
                let has = !keys.is_empty();
                self.storage_keys.write().unwrap().insert(*address, keys);
                return has;
            }
        }
        false
    }

    /// Write basic_data leaf for an account.
    ///
    /// If new code is being deployed (code is Some), uses its length for code_size.
    /// Otherwise preserves the existing code_size from the trie.
    fn write_account_info(
        &mut self,
        info: &AccountInfo,
        new_code: Option<&Code>,
        base_stem: &[u8; 31],
    ) -> Result<(), BinaryTrieError> {
        let code_size = new_code
            .map(|c| c.bytecode.len() as u32)
            .unwrap_or_else(|| {
                let basic_data_key = tree_key_from_stem(base_stem, BASIC_DATA_LEAF_KEY);
                match self.trie.get_shared(basic_data_key) {
                    Some(data) => unpack_basic_data(&data).1,
                    None => 0,
                }
            });

        let basic_data_key = tree_key_from_stem(base_stem, BASIC_DATA_LEAF_KEY);
        let basic_data = pack_basic_data(0, code_size, info.nonce, info.balance);
        self.trie.insert(basic_data_key, basic_data)?;
        self.record_insert(basic_data_key, basic_data);

        let code_hash_key = tree_key_from_stem(base_stem, CODE_HASH_LEAF_KEY);
        self.trie.insert(code_hash_key, info.code_hash.0)?;
        self.record_insert(code_hash_key, info.code_hash.0);

        Ok(())
    }

    /// Write code: chunkify into trie leaves for merkleization.
    fn write_code(
        &mut self,
        address: &Address,
        code: &Code,
        base_stem: &[u8; 31],
    ) -> Result<(), BinaryTrieError> {
        // Remove old code chunks if code_size changed.
        let basic_data_key = tree_key_from_stem(base_stem, BASIC_DATA_LEAF_KEY);
        let old_code_size = match self.trie.get_shared(basic_data_key) {
            Some(data) => unpack_basic_data(&data).1,
            None => 0,
        };
        if old_code_size > 0 {
            let old_num_chunks = (old_code_size as u64).div_ceil(31);
            let new_num_chunks = if code.bytecode.is_empty() {
                0
            } else {
                (code.bytecode.len() as u64).div_ceil(31)
            };
            for chunk_id in new_num_chunks..old_num_chunks {
                let key = get_tree_key_for_code_chunk(address, chunk_id);
                self.trie.remove(key)?;
                self.record_remove(key);
            }
        }

        // Write new code chunks.
        let chunks = chunkify_code(&code.bytecode);
        for (i, chunk) in chunks.iter().enumerate() {
            let key = get_tree_key_for_code_chunk(address, i as u64);
            self.trie.insert(key, *chunk)?;
            self.record_insert(key, *chunk);
        }

        Ok(())
    }

    /// Remove all state for an account (basic_data, code_hash, code chunks, storage).
    fn remove_account(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        // Read code_size BEFORE removing basic_data — needed to know chunk count.
        let code_size = self.get_code_size(address);

        // Remove basic_data and code_hash leaves.
        let basic_key = get_tree_key_for_basic_data(address);
        let code_hash_key = get_tree_key_for_code_hash(address);
        self.trie.remove(basic_key)?;
        self.record_remove(basic_key);
        self.trie.remove(code_hash_key)?;
        self.record_remove(code_hash_key);

        // Remove code chunks.
        if code_size > 0 {
            let num_chunks = (code_size as u64).div_ceil(31);
            for chunk_id in 0..num_chunks {
                let key = get_tree_key_for_code_chunk(address, chunk_id);
                self.trie.remove(key)?;
                self.record_remove(key);
            }
        }

        // Remove all tracked storage.
        self.clear_account_storage(address)?;

        Ok(())
    }

    /// Clear all storage slots for an account using the tracked storage_keys.
    fn clear_account_storage(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        // Ensure storage_keys are loaded if they were evicted (reload acquires write lock internally).
        if !self.storage_keys.read().unwrap().contains_key(address) {
            self.reload_storage_keys(address);
        }
        // Single write-lock acquisition: remove and retrieve the key set atomically.
        let keys = self.storage_keys.write().unwrap().remove(address);
        if let Some(keys) = keys {
            for key in keys {
                let storage_key = U256::from_big_endian(key.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(address, storage_key);
                self.trie.remove(tree_key)?;
                self.record_remove(tree_key);
            }
            self.dirty_storage_keys.insert(*address);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Backend helper loaders (used by `open`)
// ---------------------------------------------------------------------------

/// Load storage_keys from a dedicated table by iterating over all entries.
/// Keys are raw address (20 bytes); values are packed H256 storage keys.
fn load_storage_keys(
    backend: &dyn TrieBackend,
    storage_keys_table: &'static str,
) -> Result<FxHashMap<Address, FxHashSet<H256>>, BinaryTrieError> {
    let mut map: FxHashMap<Address, FxHashSet<H256>> = FxHashMap::default();
    let iter = backend.full_iterator(storage_keys_table)?;
    for (key, value) in iter {
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

    // 4. Genesis with contract; verify code_hash (no code retrieval from state).
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

        // Second account's code_hash is still in the trie.
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

    mod witness_tests {
        use super::*;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        use crate::db::{TrieBackend, WriteOp};
        use crate::error::BinaryTrieError;
        use rustc_hash::FxHashMap;
        use std::sync::Mutex;

        const NODES_TABLE: &'static str = "binary_trie_nodes";
        const STORAGE_KEYS_TABLE: &'static str = "binary_trie_storage_keys";

        fn make_address(b: u8) -> Address {
            let mut a = [0u8; 20];
            a[19] = b;
            Address::from(a)
        }

        /// Minimal in-memory TrieBackend for tests.
        #[derive(Default)]
        struct MemoryTrieBackend {
            tables: Mutex<FxHashMap<String, FxHashMap<Box<[u8]>, Vec<u8>>>>,
        }

        impl TrieBackend for MemoryTrieBackend {
            fn get(
                &self,
                table: &'static str,
                key: &[u8],
            ) -> Result<Option<Vec<u8>>, BinaryTrieError> {
                let tables = self.tables.lock().unwrap();
                Ok(tables.get(table).and_then(|t| t.get(key)).cloned())
            }

            fn write_batch(&self, ops: Vec<WriteOp>) -> Result<(), BinaryTrieError> {
                let mut tables = self.tables.lock().unwrap();
                for op in ops {
                    match op {
                        WriteOp::Put { table, key, value } => {
                            tables
                                .entry(table.to_string())
                                .or_default()
                                .insert(key, value);
                        }
                        WriteOp::Delete { table, key } => {
                            if let Some(t) = tables.get_mut(table) {
                                t.remove(key.as_ref());
                            }
                        }
                    }
                }
                Ok(())
            }

            fn full_iterator(
                &self,
                table: &'static str,
            ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, BinaryTrieError> {
                let tables = self.tables.lock().unwrap();
                let entries: Vec<(Vec<u8>, Vec<u8>)> = tables
                    .get(table)
                    .map(|t| t.iter().map(|(k, v)| (k.to_vec(), v.clone())).collect())
                    .unwrap_or_default();
                Ok(Box::new(entries.into_iter()))
            }
        }

        fn open_test_state() -> BinaryTrieState {
            let backend = Arc::new(MemoryTrieBackend::default());
            BinaryTrieState::open(backend, NODES_TABLE, STORAGE_KEYS_TABLE).unwrap()
        }

        #[test]
        fn test_generate_witness_basic() {
            let mut state = open_test_state();

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
            state.state_root(); // re-cache hashes after flush

            // Generate witness: account was accessed, no storage.
            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![]);
            let block_hash = H256::from_low_u64_be(0x5678);
            let witness = state
                .generate_witness(
                    1,
                    block_hash,
                    &accessed,
                    &HashSet::new(),
                    &HashMap::new(),
                    vec![],
                )
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
            let mut state = open_test_state();

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
            state.state_root(); // re-cache hashes after flush

            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![slot_h256]);
            let block_hash = H256::from_low_u64_be(0xBBBB);
            let witness = state
                .generate_witness(
                    1,
                    block_hash,
                    &accessed,
                    &HashSet::new(),
                    &HashMap::new(),
                    vec![],
                )
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
            let mut state = open_test_state();

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
            state.state_root(); // re-cache hashes after flush

            let mut accessed = HashMap::new();
            accessed.insert(addr, vec![]);
            let mut accessed_codes = HashSet::new();
            accessed_codes.insert(code_hash);

            // Supply the code via the codes map (simulating Store lookup).
            let mut codes = HashMap::new();
            codes.insert(code_hash, bytecode.clone());

            let block_hash = H256::from_low_u64_be(0xDDDD);
            let witness = state
                .generate_witness(1, block_hash, &accessed, &accessed_codes, &codes, vec![])
                .unwrap();

            assert_eq!(witness.codes.len(), 1);
            assert_eq!(witness.codes[0].code_hash, code_hash);
            assert_eq!(witness.codes[0].bytecode, bytecode.to_vec());

            // Account entry should have the code_hash.
            assert_eq!(witness.account_proofs[0].code_hash, code_hash);
        }
    }
}
