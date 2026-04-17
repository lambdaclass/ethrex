use std::collections::{BTreeMap, HashMap, btree_map, hash_map::Entry};
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;

use ethereum_types::{Address, H256, U256};
use ethrex_common::{
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountState, AccountUpdate, Code},
};
use ethrex_crypto::{Crypto, NativeCrypto, keccak::keccak_hash};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_state_backend::{
    AccountMut, CodeMut, CodeReader, MerkleOutput, NodeUpdates, StateCommitter, StateError,
    StateReader,
};

use crate::merkleizer::TrieProvider;
use crate::{
    EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, NodeRef, Trie, TrieError, TrieLogger, TrieWitness,
};

/// Storage tries with witness loggers attached, keyed by account address.
/// Each entry holds the witness recorder and the logged trie for one account.
pub type MptStorageTries = HashMap<Address, (TrieWitness, Trie)>;

// ---- From impls to cut boilerplate ----

impl From<TrieError> for StateError {
    fn from(e: TrieError) -> Self {
        StateError::Trie(e.to_string())
    }
}

// ---- Public MPT key derivation helpers ----

/// Keccak-hash an address to derive the MPT state trie key.
/// MPT-specific: other backends may use a different key derivation.
pub fn hash_address(address: &Address) -> [u8; 32] {
    keccak_hash(address.to_fixed_bytes())
}

/// Keccak-hash a storage key to derive the MPT storage trie key.
/// MPT-specific: other backends may use a different key derivation.
pub fn hash_key(key: &H256) -> [u8; 32] {
    keccak_hash(key.to_fixed_bytes())
}

// ---- No-op implementations for in-memory / witness paths ----

/// No-op `TrieProvider` for contexts where tries are pre-loaded.
/// Used by genesis (no existing state) and witness (all tries already in caches).
struct EmptyTrieProvider;

impl TrieProvider for EmptyTrieProvider {
    fn open_state_trie(&self, _root: H256) -> Result<Trie, TrieError> {
        Ok(Trie::default())
    }
    fn open_storage_trie(
        &self,
        _account_hash: H256,
        _storage_root: H256,
    ) -> Result<Trie, TrieError> {
        Ok(Trie::default())
    }
}

/// Returns `None` for all lookups. Used when all code is pre-loaded in the
/// `codes` map (witness path) or when no code exists yet (genesis).
fn no_op_code_reader(_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
    Ok(None)
}

// ---- Witness state ----

/// Internal state for witness recording mode.
/// Held inside `MptBackend` when the backend is in witness mode.
pub(crate) struct MptWitnessState {
    /// Arc<Mutex<...>> shared with the TrieLogger for the current state trie.
    pub(crate) trie_witness: TrieWitness,
    /// Accumulated state trie witness nodes across all blocks processed so far.
    pub(crate) accumulated_state_witness: HashMap<NodeHash, Node>,
    /// All collected trie nodes (state + storage) across all blocks.
    pub(crate) used_trie_nodes: Vec<Node>,
    /// Root node saved for empty-block fallback (used if witness is empty).
    pub(crate) root_node: Option<Arc<Node>>,
    /// Initial state root, used by finalize to build embedded tries.
    pub(crate) initial_state_root: H256,
    /// Storage tries with witness loggers, accumulated across blocks.
    pub(crate) storage_tries: MptStorageTries,
}

// ---- MptBackend ----

/// MPT-backed implementation of [`StateReader`] and [`StateCommitter`].
///
/// Storage tries are cached in `storage_tries` and lazily opened via
/// `storage_opener` on first access. The opener's `Trie` may be backed
/// by any [`TrieDB`] implementation (in-memory, layer cache + disk, etc.).
pub struct MptBackend {
    pub(crate) state_trie: Trie,
    /// Cached storage tries keyed by keccak(address). Lazily populated
    /// via `storage_opener` when a trie is first accessed.
    pub(crate) storage_tries: BTreeMap<H256, Trie>,
    /// Code cache keyed by code_hash. Checked before `code_reader`.
    pub(crate) codes: BTreeMap<H256, Code>,
    pub(crate) crypto: Arc<dyn Crypto>,
    /// Opens storage tries for accounts not yet in `storage_tries`.
    storage_opener: Arc<dyn TrieProvider>,
    /// Reads code by hash when not in the `codes` cache.
    code_reader: CodeReader,
    /// Cached storage roots to avoid re-reading accounts on repeated SLOAD.
    /// Maps hashed_address -> storage_root.
    storage_root_cache: Mutex<FxHashMap<H256, H256>>,
    /// Witness recording state. `Some` when in witness mode, `None` otherwise.
    pub(crate) witness_state: Option<MptWitnessState>,
}

impl MptBackend {
    /// Create a new `MptBackend` with the given state trie and empty caches.
    /// Uses no-op opener/reader (returns empty tries and no code).
    /// Suitable for genesis (no existing state to open from).
    pub fn new(state_trie: Trie, crypto: Arc<dyn Crypto>) -> Self {
        Self {
            state_trie,
            storage_tries: BTreeMap::new(),
            codes: BTreeMap::new(),
            crypto,
            storage_opener: Arc::new(EmptyTrieProvider),
            code_reader: Arc::new(no_op_code_reader),
            storage_root_cache: Mutex::new(FxHashMap::default()),
            witness_state: None,
        }
    }

    /// Create from pre-built tries and codes (e.g. from an execution witness).
    /// Uses no-op opener/reader since everything is pre-loaded.
    pub fn from_witness(
        state_trie: Trie,
        storage_tries: BTreeMap<H256, Trie>,
        codes: BTreeMap<H256, Code>,
        crypto: Arc<dyn Crypto>,
    ) -> Self {
        Self {
            state_trie,
            storage_tries,
            codes,
            crypto,
            storage_opener: Arc::new(EmptyTrieProvider),
            code_reader: Arc::new(no_op_code_reader),
            storage_root_cache: Mutex::new(FxHashMap::default()),
            witness_state: None,
        }
    }

    /// Create from serialized witness bytes (state_proof from ExecutionWitness).
    ///
    /// Deserializes the encoded nodes into a `BTreeMap<H256, Node>`, rebuilds the
    /// state trie rooted at `initial_state_root`, walks the state trie to find
    /// accounts with non-empty storage tries, and rebuilds those storage tries too.
    pub fn from_witness_bytes(
        state_proof: Vec<Vec<u8>>,
        initial_state_root: H256,
        codes: BTreeMap<H256, Code>,
        crypto: Arc<dyn Crypto>,
    ) -> Result<Self, StateError> {
        use ethrex_rlp::decode::RLPDecode;

        // Deserialize bytes into nodes keyed by their keccak hash.
        let nodes: BTreeMap<H256, Node> = state_proof
            .into_iter()
            .filter_map(|b| {
                if b == [0x80u8] {
                    return None; // skip RLP null
                }
                let hash = H256(keccak_hash(&b));
                Some(Node::decode(&b).map(|node| (hash, node)))
            })
            .collect::<Result<_, _>>()
            .map_err(|e: ethrex_rlp::error::RLPDecodeError| StateError::Trie(e.to_string()))?;

        // Rebuild the state trie root from the parent header's state root hash.
        let state_trie_root = if let NodeRef::Node(root, _) =
            Trie::get_embedded_root(&nodes, initial_state_root)
                .map_err(|e| StateError::Trie(e.to_string()))?
        {
            Some((*root).clone())
        } else {
            None
        };

        let state_trie = if let Some(ref root) = state_trie_root {
            Trie::new_temp_with_root(root.clone().into())
        } else {
            Trie::new_temp()
        };
        state_trie.hash_no_commit(crypto.as_ref());

        // Rebuild storage tries from the embedded nodes found in the state trie.
        let mut storage_tries = BTreeMap::new();
        if let Some(ref state_trie_root_node) = state_trie_root {
            let mut accounts = Vec::new();
            collect_accounts_from_trie(
                state_trie_root_node,
                Nibbles::from_raw(&[], false),
                &mut accounts,
                &nodes,
            );

            for (hashed_address, storage_root_hash) in accounts {
                if storage_root_hash == *EMPTY_TRIE_HASH {
                    continue;
                }
                if !nodes.contains_key(&storage_root_hash) {
                    continue;
                }
                let node = Trie::get_embedded_root(&nodes, storage_root_hash)
                    .map_err(|e| StateError::Trie(e.to_string()))?;
                let NodeRef::Node(node, _) = node else {
                    return Err(StateError::Trie(
                        "execution witness does not contain non-empty storage trie".to_string(),
                    ));
                };
                let storage_trie = Trie::new_temp_with_root((*node).clone().into());
                storage_trie.hash_no_commit(crypto.as_ref());
                storage_tries.insert(hashed_address, storage_trie);
            }
        }

        Ok(Self::from_witness(state_trie, storage_tries, codes, crypto))
    }

    /// Create an `MptBackend` backed by a store.
    /// Storage tries are opened on demand via `storage_opener`.
    /// Code is read via `code_reader` when not in the cache.
    pub fn new_with_db(
        state_trie: Trie,
        crypto: Arc<dyn Crypto>,
        storage_opener: Arc<dyn TrieProvider>,
        code_reader: CodeReader,
    ) -> Self {
        Self {
            state_trie,
            storage_tries: BTreeMap::new(),
            codes: BTreeMap::new(),
            crypto,
            storage_opener,
            code_reader,
            storage_root_cache: Mutex::new(FxHashMap::default()),
            witness_state: None,
        }
    }

    /// Keccak-hash an address to derive the MPT state trie key.
    pub fn hash_address(&self, addr: &Address) -> H256 {
        H256(self.crypto.keccak256(&addr.to_fixed_bytes()))
    }

    /// Read-only access to the crypto handle. Used by consumers that need to
    /// derive hashes consistently with this backend.
    pub fn crypto(&self) -> &Arc<dyn Crypto> {
        &self.crypto
    }

    /// Read-only access to a cached storage trie by hashed account address.
    pub fn storage_trie(&self, hashed_addr: &H256) -> Option<&Trie> {
        self.storage_tries.get(hashed_addr)
    }

    /// Read-only access to a cached contract code by hash.
    pub fn code_cached(&self, code_hash: &H256) -> Option<&Code> {
        self.codes.get(code_hash)
    }

    fn hash_slot(&self, slot: &H256) -> [u8; 32] {
        self.crypto.keccak256(&slot.to_fixed_bytes())
    }

    /// Return full AccountState including storage_root.
    /// MPT-specific: other backends may not expose a per-account storage root.
    /// Not on the StateReader trait.
    pub fn account_state(&self, addr: Address) -> Result<Option<AccountState>, StateError> {
        let hashed = self.hash_address(&addr);
        let Some(encoded) = self.state_trie.get(hashed.as_bytes())? else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
        Ok(Some(state))
    }

    /// Read a storage slot when the account's storage_root is already known.
    /// Avoids re-reading the account from the state trie.
    /// Performance optimization for the VM hot path (SLOAD).
    /// Not on the StateReader trait.
    pub fn storage_with_hint(
        &self,
        hashed_addr: H256,
        storage_root: H256,
        slot: H256,
    ) -> Result<H256, StateError> {
        if storage_root == *EMPTY_TRIE_HASH {
            return Ok(H256::zero());
        }

        let storage_trie = match self.storage_tries.get(&hashed_addr) {
            Some(trie) => trie,
            None => {
                // Not cached; open on demand (no caching here since &self is immutable)
                let trie = self
                    .storage_opener
                    .open_storage_trie(hashed_addr, storage_root)?;
                // Can't insert into cache with &self, read directly
                let hashed_slot = self.hash_slot(&slot);
                return Self::read_storage_value(&trie, &hashed_slot);
            }
        };
        let hashed_slot = self.hash_slot(&slot);
        Self::read_storage_value(storage_trie, &hashed_slot)
    }

    fn read_storage_value(trie: &Trie, hashed_slot: &[u8]) -> Result<H256, StateError> {
        let Some(encoded) = trie.get(hashed_slot)? else {
            return Ok(H256::zero());
        };
        let value = U256::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
        Ok(H256::from(value.to_big_endian()))
    }

    /// Apply account updates to the given trie while recording witness nodes.
    ///
    /// `state_trie` must already be wrapped with a [`TrieLogger`] by the caller.
    /// Storage tries in `storage_tries` are reused across calls (multi-block accumulation).
    /// Any storage trie not already present is opened via `self.storage_opener` and
    /// wrapped with a new [`TrieLogger`] before use.
    ///
    /// Returns the updated `MptStorageTries` map (for passing to the next block) and the
    /// [`MerkleOutput`] for committing the block's state changes.
    pub fn apply_updates_with_witness(
        &self,
        mut state_trie: Trie,
        account_updates: &[AccountUpdate],
        mut storage_tries: MptStorageTries,
    ) -> Result<(MptStorageTries, MerkleOutput), StateError> {
        let mut ret_storage_updates = Vec::new();
        let mut code_updates = Vec::new();

        for update in account_updates {
            let hashed_address = self.hash_address(&update.address);

            if update.removed {
                state_trie.remove(hashed_address.as_bytes())?;
                continue;
            }

            let mut account_state = match state_trie.get(hashed_address.as_bytes())? {
                Some(encoded) => {
                    AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?
                }
                None => AccountState::default(),
            };

            if update.removed_storage {
                account_state.storage_root = *EMPTY_TRIE_HASH;
            }

            if let Some(info) = &update.info {
                account_state.nonce = info.nonce;
                account_state.balance = info.balance;
                account_state.code_hash = info.code_hash;
                if let Some(code) = &update.code {
                    code_updates.push((info.code_hash, code.clone()));
                }
            }

            if !update.added_storage.is_empty() {
                // If storage was wiped, discard any pre-loaded trie to avoid
                // reusing stale nodes from a prior block's selfdestructed contract.
                // The subsequent Entry::Vacant arm then opens a fresh empty trie.
                if update.removed_storage {
                    storage_tries.remove(&update.address);
                }
                let (_witness, storage_trie) = match storage_tries.entry(update.address) {
                    Entry::Occupied(occ) => occ.into_mut(),
                    Entry::Vacant(vac) => {
                        let raw_trie = self
                            .storage_opener
                            .open_storage_trie(hashed_address, account_state.storage_root)
                            .map_err(StateError::from)?;
                        vac.insert(TrieLogger::open_trie(raw_trie))
                    }
                };

                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = self.hash_slot(storage_key);
                    if storage_value.is_zero() {
                        storage_trie.remove(&hashed_key)?;
                    } else {
                        storage_trie.insert(hashed_key.to_vec(), storage_value.encode_to_vec())?;
                    }
                }

                let (storage_hash, storage_updates) =
                    storage_trie.collect_changes_since_last_hash(self.crypto.as_ref());
                account_state.storage_root = storage_hash;
                ret_storage_updates.push((
                    hashed_address,
                    storage_updates
                        .into_iter()
                        .map(|(nib, rlp)| (nib.into_vec(), rlp))
                        .collect(),
                ));
            }

            state_trie.insert(
                hashed_address.as_bytes().to_vec(),
                account_state.encode_to_vec(),
            )?;
        }

        let (state_trie_hash, state_updates) =
            state_trie.collect_changes_since_last_hash(self.crypto.as_ref());

        let merkle_output = MerkleOutput {
            root: state_trie_hash,
            node_updates: NodeUpdates::Mpt {
                state_updates: state_updates
                    .into_iter()
                    .map(|(nib, rlp)| (nib.into_vec(), rlp))
                    .collect(),
                storage_updates: ret_storage_updates,
            },
            code_updates,
            accumulated_updates: None,
        };

        Ok((storage_tries, merkle_output))
    }

    // ---- Witness-recording methods ----

    /// Initialize witness recording mode on this backend.
    ///
    /// Wraps `self.state_trie` with a [`TrieLogger`] and sets up the
    /// internal [`MptWitnessState`]. Must be called before any other
    /// witness methods. The `initial_state_root` is the state root
    /// before any blocks in the batch are applied.
    pub fn init_witness(&mut self, initial_state_root: H256) -> Result<(), StateError> {
        let trie = std::mem::take(&mut self.state_trie);
        let root_node = trie.root_node()?;
        let (trie_witness, logged_trie) = TrieLogger::open_trie(trie);
        let initial_witness = trie_witness
            .lock()
            .map_err(|_| StateError::Trie("Failed to lock trie witness".to_string()))?
            .clone();
        self.state_trie = logged_trie;
        self.witness_state = Some(MptWitnessState {
            trie_witness,
            accumulated_state_witness: initial_witness,
            used_trie_nodes: Vec::new(),
            root_node,
            initial_state_root,
            storage_tries: HashMap::new(),
        });
        Ok(())
    }

    /// Record a witness access for a single account.
    ///
    /// Reads the account from `self.state_trie` (which is TrieLogger-wrapped)
    /// so the logger records the nodes touched.
    pub fn record_witness_account(&self, addr: &Address) -> Result<(), StateError> {
        let hashed = self.hash_address(addr);
        self.state_trie.get(hashed.as_bytes())?;
        Ok(())
    }

    /// Record witness accesses for an account's storage slots.
    ///
    /// Wraps `storage_trie` with a [`TrieLogger`], reads each slot so the
    /// logger records the nodes touched, and stores the logged trie in
    /// `witness_state.storage_tries` for reuse during `apply_witness_updates`.
    pub fn record_witness_storage(
        &mut self,
        addr: &Address,
        keys: &[H256],
        storage_trie: Trie,
    ) -> Result<(), StateError> {
        // Pre-compute hashed keys before borrowing witness_state mutably.
        let hashed_keys: Vec<[u8; 32]> = keys.iter().map(|k| self.hash_slot(k)).collect();

        let ws = self
            .witness_state
            .as_mut()
            .ok_or_else(|| StateError::Trie("Witness not initialized".to_string()))?;
        let (witness, logged_trie) = TrieLogger::open_trie(storage_trie);
        for hashed_key in &hashed_keys {
            logged_trie.get(hashed_key)?;
        }
        ws.storage_tries.insert(*addr, (witness, logged_trie));
        Ok(())
    }

    /// Apply account updates while recording witness nodes.
    ///
    /// Uses `self.state_trie` (already TrieLogger-wrapped) and the
    /// accumulated storage tries from `witness_state.storage_tries`.
    /// Returns the [`MerkleOutput`] for committing the block.
    ///
    /// After this call, `self.state_trie` is consumed (set to default).
    /// The caller must call `advance_witness` immediately after to supply
    /// the new state trie for the next block.
    pub fn apply_witness_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<MerkleOutput, StateError> {
        // Take out the storage_tries so we can pass them to apply_updates_with_witness.
        let storage_tries = {
            let ws = self
                .witness_state
                .as_mut()
                .ok_or_else(|| StateError::Trie("Witness not initialized".to_string()))?;
            std::mem::take(&mut ws.storage_tries)
        };

        // Move out the logged state trie.
        let state_trie = std::mem::take(&mut self.state_trie);

        // Now &self borrow is valid (no outstanding &mut borrows).
        let (updated_storage_tries, merkle_output) =
            self.apply_updates_with_witness(state_trie, account_updates, storage_tries)?;

        // Put the updated storage_tries back.
        let ws = self
            .witness_state
            .as_mut()
            .ok_or_else(|| StateError::Trie("Witness not initialized".to_string()))?;
        ws.storage_tries = updated_storage_tries;

        // self.state_trie is now Trie::default(). The caller must call advance_witness
        // to replace it with a new TrieLogger-wrapped trie for the next block.

        Ok(merkle_output)
    }

    /// Advance witness recording to the next block's state trie.
    ///
    /// Accumulates nodes from the current TrieLogger witness into
    /// `accumulated_state_witness`, drains storage witness nodes into
    /// `used_trie_nodes`, then replaces `self.state_trie` with a new
    /// TrieLogger-wrapped trie opened from the given `new_state_trie`.
    pub fn advance_witness(&mut self, new_state_trie: Trie) -> Result<(), StateError> {
        let ws = self
            .witness_state
            .as_mut()
            .ok_or_else(|| StateError::Trie("Witness not initialized".to_string()))?;

        // Drain storage witness nodes
        for (_addr, (witness, _trie)) in ws.storage_tries.drain() {
            let nodes = witness
                .lock()
                .map_err(|_| StateError::Trie("Failed to lock storage trie witness".to_string()))?;
            ws.used_trie_nodes.extend(nodes.values().cloned());
        }

        // Accumulate current state trie witness into accumulated_state_witness
        let current_witness = ws
            .trie_witness
            .lock()
            .map_err(|_| StateError::Trie("Failed to lock state trie witness".to_string()))?;
        for (hash, node) in current_witness.iter() {
            ws.accumulated_state_witness.insert(*hash, node.clone());
        }
        drop(current_witness);

        // Open new logged trie for the next block
        let (new_trie_witness, new_logged_trie) = TrieLogger::open_trie(new_state_trie);
        ws.trie_witness = new_trie_witness;
        self.state_trie = new_logged_trie;

        Ok(())
    }

    /// Collect bytecode for the given code hashes using the `code_reader`.
    pub fn collect_witness_codes(&self, code_hashes: &[H256]) -> Result<Vec<Vec<u8>>, StateError> {
        let mut result = Vec::with_capacity(code_hashes.len());
        for &hash in code_hashes {
            let code = (self.code_reader)(hash)?
                .ok_or_else(|| StateError::Storage(format!("Code not found for hash {hash:?}")))?;
            result.push(code);
        }
        Ok(result)
    }

    /// Finalize witness recording and serialize all accumulated nodes into state_proof bytes.
    ///
    /// Consumes `self` to extract the witness state. After calling this,
    /// the backend is no longer usable.
    pub fn finalize_witness(
        mut self,
        touched_accounts: &BTreeMap<Address, Vec<H256>>,
    ) -> Result<Vec<Vec<u8>>, StateError> {
        let ws = self
            .witness_state
            .take()
            .ok_or_else(|| StateError::Trie("Witness not initialized".to_string()))?;

        let MptWitnessState {
            trie_witness,
            accumulated_state_witness: mut acc_witness,
            mut used_trie_nodes,
            root_node,
            initial_state_root,
            storage_tries,
            ..
        } = ws;

        // Drain any remaining storage tries
        for (_addr, (witness, _trie)) in storage_tries {
            let nodes = witness
                .lock()
                .map_err(|_| StateError::Trie("Failed to lock storage trie witness".to_string()))?;
            used_trie_nodes.extend(nodes.values().cloned());
        }

        // Add the final state trie witness into accumulator
        let final_witness = trie_witness
            .lock()
            .map_err(|_| StateError::Trie("Failed to lock state trie witness".to_string()))?;
        for (hash, node) in final_witness.iter() {
            acc_witness.insert(*hash, node.clone());
        }
        drop(final_witness);

        // Combine: state witness nodes + all storage nodes
        used_trie_nodes.extend(acc_witness.into_values());

        // If the witness is empty, fall back to just the root node
        if used_trie_nodes.is_empty()
            && let Some(root) = root_node
        {
            used_trie_nodes.push((*root).clone());
        }

        // Build BTreeMap of all nodes keyed by their hash
        let nodes: BTreeMap<H256, Node> = used_trie_nodes
            .into_iter()
            .map(|node| {
                (
                    node.compute_hash(&NativeCrypto).finalize(&NativeCrypto),
                    node,
                )
            })
            .collect();

        // Get the embedded state trie root
        let state_trie_root = match Trie::get_embedded_root(&nodes, initial_state_root)? {
            NodeRef::Node(root_node, _) => Some((*root_node).clone()),
            _ => None,
        };

        // Build a temp trie to look up account storage roots
        let state_trie = match &state_trie_root {
            Some(root) => Trie::new_temp_with_root(root.clone().into()),
            None => Trie::new_temp(),
        };

        // For each touched account, find its storage root and embed
        let mut storage_trie_roots = Vec::new();
        for address in touched_accounts.keys() {
            let hashed_address = hash_address(address);
            let Some(encoded_account) = state_trie.get(&hashed_address)? else {
                continue;
            };
            let storage_root_hash = AccountState::decode(&encoded_account)
                .map_err(|e| StateError::Trie(e.to_string()))?
                .storage_root;
            if storage_root_hash == *EMPTY_TRIE_HASH {
                continue;
            }
            if !nodes.contains_key(&storage_root_hash) {
                continue;
            }
            let node_ref = Trie::get_embedded_root(&nodes, storage_root_hash)?;
            let NodeRef::Node(node, _) = node_ref else {
                return Err(StateError::Trie(
                    "execution witness does not contain non-empty storage trie".to_string(),
                ));
            };
            storage_trie_roots.push((*node).clone());
        }

        // Serialize into state_proof bytes
        let mut state_proof: Vec<Vec<u8>> = Vec::new();
        if let Some(ref root) = state_trie_root {
            root.encode_subtrie(&mut state_proof)?;
        }
        for node in &storage_trie_roots {
            node.encode_subtrie(&mut state_proof)?;
        }

        Ok(state_proof)
    }

    /// Update storage roots in the state trie for all modified storage tries.
    ///
    /// Called before computing the state root when storage changes have been applied
    /// directly to storage tries without going through `commit()`. Uses `hash_no_commit`
    /// on each storage trie so the state trie reflects current storage state.
    ///
    /// Safe in the guest path because `collect_changes_since_last_hash` is never
    /// called there, so the OnceCell poisoning from `hash_no_commit` is harmless.
    pub fn flush_storage_roots(&mut self) -> Result<(), StateError> {
        for (hashed_addr, trie) in &self.storage_tries {
            // Invariant: `storage_tries` only holds entries for accounts that
            // exist in the state trie. `update_accounts(None)` drops the
            // storage entry when removing an account. If we see a storage trie
            // without a matching account, that invariant is broken and the
            // resulting state root would silently be wrong, so fail loudly.
            let Some(encoded) = self.state_trie.get(hashed_addr.as_bytes())? else {
                return Err(StateError::Trie(format!(
                    "storage_tries entry without corresponding account leaf: {hashed_addr:?}"
                )));
            };
            let storage_root = trie.hash_no_commit(self.crypto.as_ref());
            let mut state =
                AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
            state.storage_root = storage_root;
            self.state_trie
                .insert(hashed_addr.as_bytes().to_vec(), state.encode_to_vec())?;
        }
        Ok(())
    }

    /// Return the state trie root without collecting node diffs.
    ///
    /// Unlike `hash()` on `StateCommitter` (which requires `&mut self` and is
    /// used on the write path), this method takes `&self` and is intended for
    /// read-only root queries after `flush_storage_roots()` has been called.
    pub fn hash_no_commit_state(&self) -> H256 {
        self.state_trie.hash_no_commit(self.crypto.as_ref())
    }
}

impl StateReader for MptBackend {
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError> {
        let hashed = self.hash_address(&addr);
        let Some(encoded) = self.state_trie.get(hashed.as_bytes())? else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
        Ok(Some(AccountInfo {
            nonce: state.nonce,
            balance: state.balance,
            code_hash: state.code_hash,
        }))
    }

    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError> {
        let hashed_addr = self.hash_address(&addr);

        // Check cached storage tries first
        if let Some(storage_trie) = self.storage_tries.get(&hashed_addr) {
            let hashed_slot = self.hash_slot(&slot);
            return Self::read_storage_value(storage_trie, &hashed_slot);
        }

        // Get storage_root from cache or read from state trie
        let storage_root = {
            let cache = self
                .storage_root_cache
                .lock()
                .map_err(|e| StateError::Storage(format!("storage_root_cache poisoned: {e}")))?;
            cache.get(&hashed_addr).copied()
        };
        let storage_root = match storage_root {
            Some(root) => root,
            None => {
                let Some(encoded) = self.state_trie.get(hashed_addr.as_bytes())? else {
                    return Ok(H256::zero());
                };
                let state =
                    AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
                self.storage_root_cache
                    .lock()
                    .map_err(|e| StateError::Storage(format!("storage_root_cache poisoned: {e}")))?
                    .insert(hashed_addr, state.storage_root);
                state.storage_root
            }
        };

        if storage_root == *EMPTY_TRIE_HASH {
            return Ok(H256::zero());
        }
        let storage_trie = self
            .storage_opener
            .open_storage_trie(hashed_addr, storage_root)?;
        let hashed_slot = self.hash_slot(&slot);
        Self::read_storage_value(&storage_trie, &hashed_slot)
    }

    fn code(&self, _addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(None);
        }
        // Check cache first
        if let Some(code) = self.codes.get(&code_hash) {
            return Ok(Some(code.bytecode.to_vec()));
        }
        (self.code_reader)(code_hash)
    }
}

impl StateCommitter for MptBackend {
    fn update_accounts(
        &mut self,
        addrs: &[Address],
        muts: &[AccountMut],
    ) -> Result<(), StateError> {
        for (addr, acct_mut) in addrs.iter().zip(muts.iter()) {
            let hashed = self.hash_address(addr);
            match &acct_mut.account {
                None => {
                    // Removing an account must also drop any in-memory storage
                    // trie for it. Otherwise flush_storage_roots / commit would
                    // iterate the stale entry and resurrect the account with a
                    // default AccountState, producing a wrong state root on
                    // SELFDESTRUCT.
                    self.state_trie.remove(hashed.as_bytes())?;
                    self.storage_tries.remove(&hashed);
                    self.storage_root_cache
                        .lock()
                        .map_err(|e| {
                            StateError::Storage(format!("storage_root_cache poisoned: {e}"))
                        })?
                        .remove(&hashed);
                }
                Some(info) => {
                    let mut state = match self.state_trie.get(hashed.as_bytes())? {
                        Some(encoded) => AccountState::decode(&encoded)
                            .map_err(|e| StateError::Trie(e.to_string()))?,
                        None => AccountState::default(),
                    };
                    state.nonce = info.nonce;
                    state.balance = info.balance;
                    state.code_hash = info.code_hash;
                    if let Some(CodeMut { code: Some(bytes) }) = &acct_mut.code {
                        let code =
                            Code::from_bytecode_unchecked(bytes.clone().into(), info.code_hash);
                        self.codes.insert(info.code_hash, code);
                    }
                    self.state_trie
                        .insert(hashed.as_bytes().to_vec(), state.encode_to_vec())?;
                }
            }
        }
        Ok(())
    }

    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError> {
        let crypto = Arc::clone(&self.crypto);
        let hashed_addr = self.hash_address(&addr);
        let storage_trie = match self.storage_tries.entry(hashed_addr) {
            btree_map::Entry::Occupied(e) => e.into_mut(),
            btree_map::Entry::Vacant(e) => {
                let storage_root = match self.state_trie.get(hashed_addr.as_bytes())? {
                    Some(encoded) => {
                        AccountState::decode(&encoded)
                            .map_err(|e| StateError::Trie(e.to_string()))?
                            .storage_root
                    }
                    None => *EMPTY_TRIE_HASH,
                };
                let trie = self
                    .storage_opener
                    .open_storage_trie(hashed_addr, storage_root)?;
                e.insert(trie)
            }
        };
        for (slot, value) in slots {
            let hashed_slot = crypto.keccak256(&slot.to_fixed_bytes()).to_vec();
            let value_u256 = U256::from_big_endian(value.as_bytes());
            if value_u256.is_zero() {
                storage_trie.remove(&hashed_slot)?;
            } else {
                storage_trie.insert(hashed_slot, value_u256.encode_to_vec())?;
            }
        }
        // Storage root is computed in commit() via collect_changes_since_last_hash,
        // matching main's pattern. We must NOT call hash_no_commit here because it
        // poisons OnceCell hashes via interior mutability, causing
        // collect_changes_since_last_hash to later return zero nodes.
        Ok(())
    }

    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError> {
        let hashed_addr = self.hash_address(&addr);
        // Replace storage trie with empty trie
        self.storage_tries.insert(hashed_addr, Trie::default());
        // Invalidate any cached storage_root for this address.
        self.storage_root_cache
            .lock()
            .map_err(|e| StateError::Storage(format!("storage_root_cache poisoned: {e}")))?
            .remove(&hashed_addr);
        // Update account's storage_root to empty
        if let Some(encoded) = self.state_trie.get(hashed_addr.as_bytes())? {
            let mut state =
                AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
            state.storage_root = *EMPTY_TRIE_HASH;
            self.state_trie
                .insert(hashed_addr.as_bytes().to_vec(), state.encode_to_vec())?;
        }
        Ok(())
    }

    fn hash(&mut self) -> Result<H256, StateError> {
        Ok(self.state_trie.hash_no_commit(self.crypto.as_ref()))
    }

    fn commit(mut self) -> Result<MerkleOutput, StateError> {
        // Collect storage trie node diffs and update each account's storage_root
        // in the state trie. This matches main's pattern: collect_changes_since_last_hash
        // both computes the root and collects nodes in one pass.
        let mut storage_updates = Vec::new();
        for (hashed_addr, mut trie) in self.storage_tries {
            // Same invariant as flush_storage_roots: `storage_tries` never
            // carries entries for removed accounts.
            let Some(encoded) = self.state_trie.get(hashed_addr.as_bytes())? else {
                debug_assert!(
                    false,
                    "storage_tries entry without corresponding account leaf: {hashed_addr:?}",
                );
                continue;
            };
            let (storage_root, nodes) = trie.collect_changes_since_last_hash(self.crypto.as_ref());
            let mut state =
                AccountState::decode(&encoded).map_err(|e| StateError::Trie(e.to_string()))?;
            state.storage_root = storage_root;
            self.state_trie
                .insert(hashed_addr.as_bytes().to_vec(), state.encode_to_vec())?;
            storage_updates.push((
                hashed_addr,
                nodes
                    .into_iter()
                    .map(|(nib, rlp)| (nib.into_vec(), rlp))
                    .collect(),
            ));
        }

        // Collect state trie node diffs (after storage roots are updated)
        let (root, state_nodes) = self
            .state_trie
            .collect_changes_since_last_hash(self.crypto.as_ref());
        let state_updates = state_nodes
            .into_iter()
            .map(|(nib, rlp)| (nib.into_vec(), rlp))
            .collect();

        Ok(MerkleOutput {
            root,
            node_updates: NodeUpdates::Mpt {
                state_updates,
                storage_updates,
            },
            code_updates: self.codes.into_iter().collect(),
            accumulated_updates: None,
        })
    }
}

/// Recursively walks an embedded state trie node and collects
/// `(hashed_address, storage_root)` pairs from leaf nodes.
fn collect_accounts_from_trie(
    node: &Node,
    path: Nibbles,
    accounts: &mut Vec<(H256, H256)>,
    nodes: &BTreeMap<H256, Node>,
) {
    match node {
        Node::Branch(branch) => {
            for (i, child) in branch.choices.iter().enumerate() {
                let child_node: Option<&Node> = match child {
                    NodeRef::Node(n, _) => Some(n),
                    NodeRef::Hash(hash) if hash.is_valid() => {
                        nodes.get(&hash.finalize(&NativeCrypto))
                    }
                    _ => None,
                };
                if let Some(child_node) = child_node {
                    collect_accounts_from_trie(
                        child_node,
                        path.append_new(i as u8),
                        accounts,
                        nodes,
                    );
                }
            }
        }
        Node::Extension(ext) => {
            let child_node: Option<&Node> = match &ext.child {
                NodeRef::Node(n, _) => Some(n),
                NodeRef::Hash(hash) if hash.is_valid() => nodes.get(&hash.finalize(&NativeCrypto)),
                _ => None,
            };
            if let Some(child_node) = child_node {
                collect_accounts_from_trie(child_node, path.concat(&ext.prefix), accounts, nodes);
            }
        }
        Node::Leaf(leaf) => {
            let full_path = path.concat(&leaf.partial);
            let path_bytes = full_path.to_bytes();
            if path_bytes.len() == 32 {
                let hashed_address = H256::from_slice(&path_bytes);
                match AccountState::decode(&leaf.value) {
                    Ok(account_state) => {
                        accounts.push((hashed_address, account_state.storage_root));
                    }
                    Err(e) => {
                        tracing::debug!(
                            ?hashed_address,
                            error = %e,
                            "Skipping leaf with un-decodable account state"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    path_len = path_bytes.len(),
                    "Skipping leaf with unexpected path length (expected 32)"
                );
            }
        }
    }
}
