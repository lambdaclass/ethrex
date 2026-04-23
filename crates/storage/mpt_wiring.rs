//! MPT-specific wiring: trie opening, hash helpers, snap sync support.

use crate::{
    EngineType, StateBackend, Store,
    api::{
        StorageBackend,
        tables::{
            ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, EXECUTION_WITNESSES, MISC_VALUES,
            STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
        },
    },
    error::StoreError,
    trie::{BackendTrieDB, BackendTrieDBLocked},
};
use ethrex_common::types::block_execution_witness::{ExecutionWitness, RpcExecutionWitness};
use ethrex_common::{
    Address, H256, U256,
    types::{AccountInfo, AccountState, BlockHash, Genesis, GenesisAccount, code_hash},
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::decode::RLPDecode;
use ethrex_state_backend::{AccountMut, CodeMut, CodeReader, StateCommitter, StateError};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, Trie, TrieError, TrieProvider, genesis_block};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, RwLock, mpsc::TryRecvError},
};
use tracing::{debug, error, info};

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Storage trie nodes grouped by account address hash.
///
/// Each entry contains the hashed account address and the trie nodes
/// for that account's storage trie.
pub type StorageTrieNodes = Vec<(H256, Vec<(Vec<u8>, Vec<u8>)>)>;

// ---------------------------------------------------------------------------
// Hash helpers
// ---------------------------------------------------------------------------

pub(crate) fn hash_address_fixed(address: &Address) -> H256 {
    H256::from_slice(&ethrex_trie::hash_address(address))
}

// ---------------------------------------------------------------------------
// Backend helper
// ---------------------------------------------------------------------------

pub(crate) fn state_trie_locked_backend(
    backend: &dyn StorageBackend,
    last_written: Vec<u8>,
) -> Result<BackendTrieDBLocked, StoreError> {
    // No address prefix for state trie
    BackendTrieDBLocked::new(backend, last_written)
}

// ---------------------------------------------------------------------------
// Disk commit helper
// ---------------------------------------------------------------------------

/// Key length of an account-trie leaf node's nibble path: 64 nibbles
/// (keccak(addr)) + 1 terminator byte.
const MPT_ACCOUNT_LEAF_KEY_LEN: usize = 65;
/// Key length of a storage-trie leaf when the entry is prefixed with the
/// hashed account address and suffixed with a terminator:
/// 3 prefix bytes + 64 address nibbles + 64 slot nibbles.
const MPT_STORAGE_LEAF_KEY_LEN: usize = 131;

/// Write committed MPT nodes to the appropriate disk tables.
///
/// Each node's destination table is determined by key length:
/// - `MPT_ACCOUNT_LEAF_KEY_LEN`: account leaf  -> `ACCOUNT_FLATKEYVALUE`
/// - `MPT_STORAGE_LEAF_KEY_LEN`: storage leaf -> `STORAGE_FLATKEYVALUE`
/// - shorter than account leaf: account node  -> `ACCOUNT_TRIE_NODES`
/// - anything else: storage node  -> `STORAGE_TRIE_NODES`
///
/// Leaf nodes whose key is greater than `last_written` are skipped because
/// the flat-key-value generator will produce them separately.
pub(crate) fn mpt_commit_nodes_to_disk(
    backend: &dyn StorageBackend,
    nodes: Vec<(Vec<u8>, Vec<u8>)>,
    last_written: Vec<u8>,
) -> Result<(), StoreError> {
    let mut write_tx = backend.begin_write()?;
    let mut result = Ok(());
    for (key, value) in nodes {
        let is_leaf =
            key.len() == MPT_ACCOUNT_LEAF_KEY_LEN || key.len() == MPT_STORAGE_LEAF_KEY_LEN;
        let is_account = key.len() <= MPT_ACCOUNT_LEAF_KEY_LEN;

        if is_leaf && key > last_written {
            continue;
        }
        let table = if is_leaf {
            if is_account {
                &ACCOUNT_FLATKEYVALUE
            } else {
                &STORAGE_FLATKEYVALUE
            }
        } else if is_account {
            &ACCOUNT_TRIE_NODES
        } else {
            &STORAGE_TRIE_NODES
        };
        if value.is_empty() {
            result = write_tx.delete(table, &key);
        } else {
            result = write_tx.put(table, &key, &value);
        }
        if result.is_err() {
            break;
        }
    }
    if result.is_ok() {
        result = write_tx.commit();
    }
    result
}

/// Flatten MPT node updates into prefixed byte KV pairs for the TrieLayerCache.
pub(crate) fn build_mpt_cache_layer(
    state_updates: Vec<(Vec<u8>, Vec<u8>)>,
    storage_updates: StorageTrieNodes,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    storage_updates
        .into_iter()
        .flat_map(|(account_hash, nodes)| {
            nodes
                .into_iter()
                .map(move |(path, node)| (mpt_apply_prefix(Some(account_hash), path), node))
        })
        .chain(state_updates)
        .collect()
}

// ---------------------------------------------------------------------------
// TrieProvider backed by Store
// ---------------------------------------------------------------------------

/// [`TrieProvider`] implementation backed by a [`Store`].
///
/// Created by [`Store::make_trie_provider`] and passed to
/// [`ethrex_trie::MptMerkleizer::new`] and `MptBackend::new_with_db`.
struct StoreTrieProvider {
    store: Store,
    parent_state_root: H256,
}

impl TrieProvider for StoreTrieProvider {
    fn open_state_trie(&self, root: H256) -> Result<Trie, TrieError> {
        self.store
            .open_state_trie(root)
            .map_err(|e| TrieError::DbError(anyhow::Error::new(e)))
    }

    fn open_storage_trie(&self, account_hash: H256, storage_root: H256) -> Result<Trie, TrieError> {
        self.store
            .open_storage_trie(account_hash, self.parent_state_root, storage_root)
            .map_err(|e| TrieError::DbError(anyhow::Error::new(e)))
    }
}

// ---------------------------------------------------------------------------
// impl Store: trie-opening methods
// ---------------------------------------------------------------------------

impl Store {
    /// Obtain a state trie from the given state root.
    /// Doesn't check if the state root is valid.
    /// Used for internal store operations.
    pub fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = MptTrieWrapper::new(
            state_root,
            self.trie_cache
                .read()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            Box::new(BackendTrieDB::new_for_accounts(
                self.backend.clone(),
                self.last_written()?,
            )?),
            None,
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a state trie from the given state root.
    /// Doesn't check if the state root is valid.
    /// Used for internal store operations.
    pub fn open_direct_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        Ok(Trie::open(
            Box::new(BackendTrieDB::new_for_accounts(
                self.backend.clone(),
                self.last_written()?,
            )?),
            state_root,
        ))
    }

    /// Obtain a state trie locked for reads from the given state root.
    /// Doesn't check if the state root is valid.
    /// Used for internal store operations.
    pub fn open_locked_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_db = MptTrieWrapper::new(
            state_root,
            self.trie_cache
                .read()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            None,
        );
        Ok(Trie::open(Box::new(trie_db), state_root))
    }

    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored.
    pub fn open_storage_trie(
        &self,
        account_hash: H256,
        state_root: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = MptTrieWrapper::new(
            state_root,
            self.trie_cache
                .read()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            Box::new(BackendTrieDB::new_for_storages(
                self.backend.clone(),
                self.last_written()?,
            )?),
            Some(account_hash),
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    /// Open a state trie using pre-acquired shared resources.
    /// Avoids redundant RwLock acquisitions when multiple tries are opened
    /// Obtain a storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored.
    pub fn open_direct_storage_trie(
        &self,
        account_hash: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        Ok(Trie::open(
            Box::new(BackendTrieDB::new_for_account_storage(
                self.backend.clone(),
                account_hash,
                self.last_written()?,
            )?),
            storage_root,
        ))
    }

    /// Obtain a read-locked storage trie from the given address and storage_root.
    /// Doesn't check if the account is stored.
    pub fn open_locked_storage_trie(
        &self,
        account_hash: H256,
        state_root: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let trie_db = MptTrieWrapper::new(
            state_root,
            self.trie_cache
                .read()
                .map_err(|_| StoreError::LockError)?
                .clone(),
            Box::new(state_trie_locked_backend(
                self.backend.as_ref(),
                self.last_written()?,
            )?),
            Some(account_hash),
        );
        Ok(Trie::open(Box::new(trie_db), storage_root))
    }

    // ---------------------------------------------------------------------------
    // Factory methods
    // ---------------------------------------------------------------------------

    /// Create a [`TrieProvider`] that opens state and storage tries from the
    /// store, rooted at `parent_state_root`.
    ///
    /// Passed to [`ethrex_trie::MptMerkleizer::new`] and `MptBackend::new_with_db`.
    pub fn make_trie_provider(&self, parent_state_root: H256) -> Arc<dyn TrieProvider> {
        Arc::new(StoreTrieProvider {
            store: self.clone(),
            parent_state_root,
        })
    }

    /// Create a code reader closure for the given store.
    pub(crate) fn make_code_reader(&self) -> CodeReader {
        let store = self.clone();
        Arc::new(move |code_hash| {
            store
                .get_account_code(code_hash)
                .map(|opt| opt.map(|code| code.bytecode.to_vec()))
                .map_err(|e| StateError::Storage(e.to_string()))
        })
    }

    // ---------------------------------------------------------------------------
    // State reader + StateBackend
    // ---------------------------------------------------------------------------

    /// Create a [`StateBackend`] rooted at the given state root.
    /// All reads go through [`StateReader`] methods; key derivation is
    /// internal to the backend.
    pub fn new_state_reader(&self, state_root: H256) -> Result<StateBackend, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let provider = self.make_trie_provider(state_root);
        let code_reader = self.make_code_reader();
        Ok(StateBackend::new_mpt_with_db(
            state_trie,
            Arc::new(NativeCrypto),
            provider,
            code_reader,
        ))
    }

    /// Create a witness-recording [`StateBackend`] rooted at the given state root.
    ///
    /// The returned backend has a [`TrieLogger`]-wrapped state trie that records
    /// all node accesses for proof generation. Use the witness methods on
    /// [`StateBackend`] to record accesses and finalize the witness.
    pub fn new_witness_recorder(&self, state_root: H256) -> Result<StateBackend, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let provider = self.make_trie_provider(state_root);
        let code_reader = self.make_code_reader();
        let mut backend = StateBackend::new_mpt_with_db(
            state_trie,
            Arc::new(NativeCrypto),
            provider,
            code_reader,
        );
        backend
            .init_witness(state_root)
            .map_err(|e| StoreError::Custom(e.to_string()))?;
        Ok(backend)
    }

    /// Create an in-memory [`StateBackend`] for bulk state initialization (genesis).
    ///
    /// The returned backend has no DB-backed storage opener, so
    /// [`StateCommitter::update_storage`] works on fresh in-memory tries.
    /// Do NOT use this for incremental updates against an existing DB state.
    pub fn new_state_writer(&self) -> Result<StateBackend, StoreError> {
        Ok(StateBackend::new_mpt(
            Trie::default(),
            Arc::new(NativeCrypto),
        ))
    }

    /// Write MPT trie node diffs directly to disk, bypassing TrieLayerCache.
    /// Used by genesis setup (before any block execution).
    pub(crate) fn write_mpt_node_updates(
        &self,
        state_updates: Vec<(Vec<u8>, Vec<u8>)>,
        storage_updates: StorageTrieNodes,
    ) -> Result<(), StoreError> {
        let account_trie_nodes: Vec<(Vec<u8>, Vec<u8>)> = state_updates
            .into_iter()
            .map(|(path, node)| (mpt_apply_prefix(None, path), node))
            .collect();

        let storage_trie_nodes: Vec<(Vec<u8>, Vec<u8>)> = storage_updates
            .into_iter()
            .flat_map(|(account_hash, nodes)| {
                nodes
                    .into_iter()
                    .map(move |(path, node)| (mpt_apply_prefix(Some(account_hash), path), node))
            })
            .collect();

        let mut tx = self.backend.begin_write()?;
        tx.put_batch(ACCOUNT_TRIE_NODES, account_trie_nodes)?;
        tx.put_batch(STORAGE_TRIE_NODES, storage_trie_nodes)?;
        tx.commit()?;
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // State trie access
    // ---------------------------------------------------------------------------

    /// Obtain the storage trie for the given block.
    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(self.open_state_trie(header.state_root)?))
    }

    /// Creates a new state trie with an empty state root, for testing purposes only.
    pub fn new_state_trie_for_test(&self) -> Result<Trie, StoreError> {
        self.open_state_trie(*EMPTY_TRIE_HASH)
    }

    // ---------------------------------------------------------------------------
    // Iterator methods
    // ---------------------------------------------------------------------------

    /// Returns an iterator across all accounts in the state trie given by the state_root.
    /// Does not check that the state_root is valid.
    pub fn iter_accounts_from(
        &self,
        state_root: H256,
        starting_address: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        let mut iter = self.open_locked_state_trie(state_root)?.into_iter();
        iter.advance(starting_address.0.to_vec())?;
        Ok(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), AccountState::decode(&value).ok()?))
        }))
    }

    /// Returns an iterator across all accounts in the state trie given by the state_root.
    /// Does not check that the state_root is valid.
    pub fn iter_accounts(
        &self,
        state_root: H256,
    ) -> Result<impl Iterator<Item = (H256, AccountState)>, StoreError> {
        self.iter_accounts_from(state_root, H256::zero())
    }

    /// Returns an iterator across all storage slots for the given account in the state trie.
    /// Does not check that the state_root is valid.
    pub fn iter_storage_from(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_slot: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        let state_trie = self.open_locked_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let mut iter = self
            .open_locked_storage_trie(hashed_address, state_root, storage_root)?
            .into_iter();
        iter.advance(starting_slot.0.to_vec())?;
        Ok(Some(iter.content().map_while(|(path, value)| {
            Some((H256::from_slice(&path), U256::decode(&value).ok()?))
        })))
    }

    /// Returns an iterator across all storage slots for the given account in the state trie.
    /// Does not check that the state_root is valid.
    pub fn iter_storage(
        &self,
        state_root: H256,
        hashed_address: H256,
    ) -> Result<Option<impl Iterator<Item = (H256, U256)>>, StoreError> {
        self.iter_storage_from(state_root, hashed_address, H256::zero())
    }

    // ---------------------------------------------------------------------------
    // FlatKeyValue helpers
    // ---------------------------------------------------------------------------

    /// Adds all genesis accounts and returns the genesis block's state_root.
    ///
    /// Uses [`StateCommitter`] trait methods so the logic is backend-agnostic.
    /// Trie node diffs are written directly to disk (bypasses TrieLayerCache
    /// since genesis runs before any block execution).
    pub async fn setup_genesis_state_trie(
        &self,
        genesis_accounts: BTreeMap<Address, GenesisAccount>,
    ) -> Result<H256, StoreError> {
        let mut backend = self.new_state_writer()?;

        for (address, account) in &genesis_accounts {
            let ch = code_hash(&account.code, &NativeCrypto);

            let acct_mut = AccountMut {
                account: Some(AccountInfo {
                    nonce: account.nonce,
                    balance: account.balance,
                    code_hash: ch,
                }),
                code: if account.code.is_empty() {
                    None
                } else {
                    Some(CodeMut {
                        code: Some(account.code.to_vec()),
                    })
                },
                code_size: account.code.len(),
            };

            backend.update_accounts(&[*address], &[acct_mut])?;

            if !account.storage.is_empty() {
                let slots: Vec<(H256, H256)> = account
                    .storage
                    .iter()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(k, v)| (H256(k.to_big_endian()), H256(v.to_big_endian())))
                    .collect();
                if !slots.is_empty() {
                    backend.update_storage(*address, &slots)?;
                }
            }
        }

        let output = backend.commit()?;
        let state_root = output.root;

        self.write_node_updates_direct(output.node_updates)?;
        self.write_account_code_batch(output.code_updates).await?;

        Ok(state_root)
    }

    // ---------------------------------------------------------------------------
    // Witness-aware account updates
    // ---------------------------------------------------------------------------
    // AccountState query methods
    // ---------------------------------------------------------------------------

    /// Returns the raw `AccountState` (including `storage_root`) for an account
    /// looked up by its pre-hashed key. Kept `pub` because snap-sync in
    /// `ethrex-p2p` needs `storage_root` to open per-account storage tries.
    pub fn get_account_state_by_acc_hash(
        &self,
        block_hash: BlockHash,
        account_hash: H256,
    ) -> Result<Option<AccountState>, StoreError> {
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let Some(encoded_state) = state_trie.get(account_hash.as_bytes())? else {
            return Ok(None);
        };
        let account_state = AccountState::decode(&encoded_state)?;
        Ok(Some(account_state))
    }

    // ---------------------------------------------------------------------------
    // Proof methods
    // ---------------------------------------------------------------------------

    /// Constructs a merkle proof for the given account address against a given state.
    /// If storage_keys are provided, also constructs the storage proofs for those keys.
    ///
    /// Returns `None` if the state trie is missing, otherwise returns the proof.
    pub async fn get_account_proof(
        &self,
        state_root: H256,
        address: Address,
        storage_keys: &[H256],
    ) -> Result<Option<AccountProof>, StoreError> {
        // TODO: check state root
        // let Some(state_trie) = self.open_state_trie(state_trie)? else {
        //     return Ok(None);
        // };
        let state_trie = self.open_state_trie(state_root)?;
        let address_path = hash_address_fixed(&address);
        let proof = state_trie.get_proof(address_path.as_bytes())?;
        let account_opt = state_trie
            .get(address_path.as_bytes())?
            .map(|encoded_state| AccountState::decode(&encoded_state))
            .transpose()?;

        let mut storage_proof = Vec::with_capacity(storage_keys.len());

        if let Some(account) = &account_opt {
            let storage_trie =
                self.open_storage_trie(address_path, state_root, account.storage_root)?;

            for key in storage_keys {
                let hashed_key = ethrex_trie::hash_key(key);
                let proof = storage_trie.get_proof(&hashed_key)?;
                let value = storage_trie
                    .get(&hashed_key)?
                    .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
                    .transpose()?
                    .unwrap_or_default();

                let slot_proof = StorageSlotProof {
                    proof,
                    key: *key,
                    value,
                };
                storage_proof.push(slot_proof);
            }
        } else {
            storage_proof.extend(storage_keys.iter().map(|key| StorageSlotProof {
                proof: Vec::new(),
                key: *key,
                value: U256::zero(),
            }));
        }
        let account = account_opt.unwrap_or_default();
        let account_proof = AccountProof {
            proof,
            info: AccountInfo {
                nonce: account.nonce,
                balance: account.balance,
                code_hash: account.code_hash,
            },
            storage_root: account.storage_root,
            storage_proof,
        };
        Ok(Some(account_proof))
    }

    pub fn get_account_range_proof(
        &self,
        state_root: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let mut proof = state_trie.get_proof(starting_hash.as_bytes())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&state_trie.get_proof(last_hash.as_bytes())?);
        }
        Ok(proof)
    }

    pub fn get_storage_range_proof(
        &self,
        state_root: H256,
        hashed_address: H256,
        starting_hash: H256,
        last_hash: Option<H256>,
    ) -> Result<Option<Vec<Vec<u8>>>, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let Some(account_rlp) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let storage_root = AccountState::decode(&account_rlp)?.storage_root;
        let storage_trie = self.open_storage_trie(hashed_address, state_root, storage_root)?;
        let mut proof = storage_trie.get_proof(starting_hash.as_bytes())?;
        if let Some(last_hash) = last_hash {
            proof.extend_from_slice(&storage_trie.get_proof(last_hash.as_bytes())?);
        }
        Ok(Some(proof))
    }

    /// Receives the root of the state trie and a list of paths where the first path will correspond to a path in the state trie
    /// (aka a hashed account address) and the following paths will be paths in the account's storage trie (aka hashed storage keys)
    /// If only one hash (account) is received, then the state trie node containing the account will be returned.
    /// If more than one hash is received, then the storage trie nodes where each storage key is stored will be returned
    /// For more information check out snap capability message [`GetTrieNodes`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#gettrienodes-0x06)
    /// The paths can be either full paths (hash) or partial paths (compact-encoded nibbles), if a partial path is given for the account this method will not return storage nodes for it
    pub fn get_trie_nodes(
        &self,
        state_root: H256,
        paths: Vec<Vec<u8>>,
        byte_limit: u64,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let Some(account_path) = paths.first() else {
            return Ok(vec![]);
        };
        let state_trie = self.open_state_trie(state_root)?;
        // State Trie Nodes Request
        if paths.len() == 1 {
            // Fetch state trie node
            let node = state_trie.get_node(account_path)?;
            return Ok(vec![node]);
        }
        // Storage Trie Nodes Request
        let Some(account_state) = state_trie
            .get(account_path)?
            .map(|ref rlp| AccountState::decode(rlp))
            .transpose()?
        else {
            return Ok(vec![]);
        };
        // We can't access the storage trie without the account's address hash
        let Ok(hashed_address) = account_path.clone().try_into().map(H256) else {
            return Ok(vec![]);
        };
        let storage_trie =
            self.open_storage_trie(hashed_address, state_root, account_state.storage_root)?;
        // Fetch storage trie nodes
        let mut nodes = vec![];
        let mut bytes_used = 0;
        for path in paths.iter().skip(1) {
            if bytes_used >= byte_limit {
                break;
            }
            let node = storage_trie.get_node(path)?;
            bytes_used += node.len() as u64;
            nodes.push(node);
        }
        Ok(nodes)
    }

    // ---------------------------------------------------------------------------
    // State root check
    // ---------------------------------------------------------------------------

    pub fn has_state_root(&self, state_root: H256) -> Result<bool, StoreError> {
        // Empty state trie is always available
        if state_root == *EMPTY_TRIE_HASH {
            return Ok(true);
        }
        let trie = self.open_state_trie(state_root)?;
        // NOTE: here we hash the root because the trie doesn't check the state root is correct
        let Some(root) = trie.db().get(Nibbles::default())? else {
            return Ok(false);
        };
        let root_hash = Node::decode(&root)?
            .compute_hash(&NativeCrypto)
            .finalize(&NativeCrypto);
        Ok(state_root == root_hash)
    }

    /// CAUTION: This method writes directly to the underlying database, bypassing any caching layer.
    /// For updating the state after block execution, use [`Self::store_block_updates`].
    pub async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: StorageTrieNodes,
    ) -> Result<(), StoreError> {
        let mut txn = self.backend.begin_write()?;
        tokio::task::spawn_blocking(move || {
            for (address_hash, nodes) in storage_trie_nodes {
                for (node_path, node_data) in nodes {
                    let key = mpt_apply_prefix(Some(address_hash), node_path);
                    if node_data.is_empty() {
                        txn.delete(STORAGE_TRIE_NODES, &key)?;
                    } else {
                        txn.put(STORAGE_TRIE_NODES, &key, &node_data)?;
                    }
                }
            }
            txn.commit()
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    /// Stores a pre-serialized execution witness for a block.
    ///
    /// The witness is converted to RPC format (RpcExecutionWitness) before storage
    /// to avoid expensive `encode_subtrie` traversal on every read. This pre-computes
    /// the serialization at write time instead of read time.
    pub fn store_witness(
        &self,
        block_hash: BlockHash,
        block_number: u64,
        witness: ExecutionWitness,
    ) -> Result<(), StoreError> {
        // Convert to RPC format once at storage time
        let rpc_witness = RpcExecutionWitness::from(witness);
        let key = Self::make_witness_key(block_number, &block_hash);
        let value = serde_json::to_vec(&rpc_witness)?;
        self.write(EXECUTION_WITNESSES, key, value)?;
        // Clean up old witnesses (keep only last 128)
        self.cleanup_old_witnesses(block_number)
    }

    fn cleanup_old_witnesses(&self, latest_block_number: u64) -> Result<(), StoreError> {
        // If we have less than 128 blocks, no cleanup needed
        if latest_block_number <= crate::store::MAX_WITNESSES {
            return Ok(());
        }

        let threshold = latest_block_number - crate::store::MAX_WITNESSES;

        if let Some(oldest_block_number) = self.get_oldest_witness_number()? {
            let prefix = oldest_block_number.to_be_bytes();
            let mut to_delete = Vec::new();

            {
                let read_txn = self.backend.begin_read()?;
                let iter = read_txn.prefix_iterator(EXECUTION_WITNESSES, &prefix)?;

                // We may have multiple witnesses for the same block number (forks)
                for item in iter {
                    let (key, _value) = item?;
                    let mut block_number_bytes = [0u8; 8];
                    block_number_bytes.copy_from_slice(&key[0..8]);
                    let block_number = u64::from_be_bytes(block_number_bytes);
                    if block_number > threshold {
                        break;
                    }
                    to_delete.push(key.to_vec());
                }
            }

            for key in to_delete {
                self.delete(EXECUTION_WITNESSES, key)?;
            }
        };

        self.update_oldest_witness_number(threshold + 1)?;

        Ok(())
    }

    fn update_oldest_witness_number(&self, oldest_block_number: u64) -> Result<(), StoreError> {
        self.write(
            MISC_VALUES,
            b"oldest_witness_block_number".to_vec(),
            oldest_block_number.to_le_bytes().to_vec(),
        )?;
        Ok(())
    }

    fn get_oldest_witness_number(&self) -> Result<Option<u64>, StoreError> {
        let Some(value) = self.read(MISC_VALUES, b"oldest_witness_block_number".to_vec())? else {
            return Ok(None);
        };

        let array: [u8; 8] = value.as_slice().try_into().map_err(|_| {
            StoreError::Custom("Invalid oldest witness block number bytes".to_string())
        })?;
        Ok(Some(u64::from_le_bytes(array)))
    }

    /// Obtain the storage trie for the given account on the given block
    pub fn storage_trie(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<Trie>, StoreError> {
        let Some(header) = self.get_block_header_by_hash(block_hash)? else {
            return Ok(None);
        };
        // Fetch Account from state_trie
        let Some(state_trie) = self.state_trie(block_hash)? else {
            return Ok(None);
        };
        let hashed_address = hash_address_fixed(&address);
        let Some(encoded_account) = state_trie.get(hashed_address.as_bytes())? else {
            return Ok(None);
        };
        let account = AccountState::decode(&encoded_account)?;
        // Open storage_trie
        let storage_root = account.storage_root;
        Ok(Some(self.open_storage_trie(
            hashed_address,
            header.state_root,
            storage_root,
        )?))
    }

    pub async fn new_from_genesis(
        store_path: &Path,
        engine_type: EngineType,
        genesis_path: &str,
    ) -> Result<Self, StoreError> {
        let file = std::fs::File::open(genesis_path)
            .map_err(|error| StoreError::Custom(format!("Failed to open genesis file: {error}")))?;
        let reader = std::io::BufReader::new(file);
        let genesis: Genesis = serde_json::from_reader(reader)
            .map_err(|e| StoreError::Custom(format!("Failed to deserialize genesis file: {e}")))?;
        let mut store = Self::new(
            store_path,
            engine_type,
            ethrex_state_backend::BackendKind::Mpt,
        )?;
        store.add_initial_state(genesis).await?;
        Ok(store)
    }

    pub async fn add_initial_state(&mut self, genesis: Genesis) -> Result<(), StoreError> {
        debug!("Storing initial state from genesis");

        // Obtain genesis block
        let genesis_block = genesis_block(&genesis);
        let genesis_block_number = genesis_block.header.number;

        let genesis_hash = genesis_block.hash();

        // Set chain config
        self.set_chain_config(&genesis.config).await?;

        // The cache can't be empty
        if let Some(number) = self.load_latest_block_number().await? {
            let latest_block_header = self
                .load_block_header(number)?
                .ok_or_else(|| StoreError::MissingLatestBlockNumber)?;
            self.latest_block_header.update(latest_block_header);
        }

        match self.load_block_header(genesis_block_number)? {
            Some(header) if header.hash() == genesis_hash => {
                info!("Received genesis file matching a previously stored one, nothing to do");
                return Ok(());
            }
            Some(_) => {
                error!(
                    "The chain configuration stored in the database is incompatible with the provided configuration. If you intended to switch networks, choose another datadir or clear the database (e.g., run `ethrex removedb`) and try again."
                );
                return Err(StoreError::IncompatibleChainConfig);
            }
            None => {
                self.add_block_header(genesis_hash, genesis_block.header.clone())
                    .await?
            }
        }
        // Store genesis accounts
        // TODO: Should we use this root instead of computing it before the block hash check?
        let genesis_state_root = self.setup_genesis_state_trie(genesis.alloc).await?;
        debug_assert_eq!(genesis_state_root, genesis_block.header.state_root);

        // Store genesis block
        info!(hash = %genesis_hash, "Storing genesis block");

        self.add_block(genesis_block).await?;
        self.update_earliest_block_number(genesis_block_number)
            .await?;
        self.forkchoice_update(vec![], genesis_block_number, genesis_hash, None, None)
            .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public proof types
// ---------------------------------------------------------------------------

pub struct AccountProof {
    pub proof: Vec<Vec<u8>>,
    pub info: AccountInfo,
    /// Storage root hash for this account (part of the Ethereum proof protocol).
    pub storage_root: H256,
    pub storage_proof: Vec<StorageSlotProof>,
}

pub struct StorageSlotProof {
    pub proof: Vec<Vec<u8>>,
    pub key: H256,
    pub value: U256,
}

// ---------------------------------------------------------------------------
// FlatKeyValue generator
// ---------------------------------------------------------------------------

/// Control messages for the FlatKeyValue generator.
#[derive(Debug, PartialEq)]
pub(crate) enum FKVGeneratorControlMessage {
    Stop,
    Continue,
}

// NOTE: we don't receive `Store` here to avoid cyclic dependencies
// with the other end of `control_rx`
pub(crate) fn flatkeyvalue_generator(
    backend: &Arc<dyn StorageBackend>,
    last_computed_fkv: &RwLock<Vec<u8>>,
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    info!("Generation of FlatKeyValue started.");
    let initial_last_written = backend
        .begin_read()?
        .get(MISC_VALUES, "last_written".as_bytes())?
        .unwrap_or_default();

    if initial_last_written.is_empty() {
        // First time generating the FKV. Remove all FKV entries just in case
        backend.clear_table(ACCOUNT_FLATKEYVALUE)?;
        backend.clear_table(STORAGE_FLATKEYVALUE)?;
    } else if initial_last_written == [0xff] {
        // FKV was already generated
        info!("FlatKeyValue already generated. Skipping.");
        return Ok(());
    }

    loop {
        // Acquire a fresh read view per iteration so updates performed while the
        // generator is paused are visible after a Continue signal.
        let read_tx = backend.begin_read()?;
        let root = read_tx
            .get(ACCOUNT_TRIE_NODES, &[])?
            .ok_or(StoreError::MissingLatestBlockNumber)?;
        let root: Node = ethrex_trie::Node::decode(&root)?;
        let state_root = root.compute_hash(&NativeCrypto).finalize(&NativeCrypto);

        let last_written = read_tx
            .get(MISC_VALUES, "last_written".as_bytes())?
            .unwrap_or_default();
        let last_written_account = last_written
            .get(0..64)
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();
        let mut last_written_storage = last_written
            .get(66..130)
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();

        debug!("Starting FlatKeyValue loop pivot={last_written:?} SR={state_root:x}");

        let mut ctr = 0;
        let mut write_txn = backend.begin_write()?;
        let mut iter = Trie::open(
            Box::new(BackendTrieDB::new_for_accounts_with_view(
                backend.clone(),
                read_tx.clone(),
                last_written.clone(),
            )?),
            state_root,
        )
        .into_iter();
        if last_written_account > Nibbles::default() {
            iter.advance(last_written_account.to_bytes())?;
        }
        let res = iter.try_for_each(|(path, node)| -> Result<(), StoreError> {
            let Node::Leaf(node) = node else {
                return Ok(());
            };
            let account_state = AccountState::decode(&node.value)?;
            let account_hash = H256::from_slice(&path.to_bytes());
            write_txn.put(MISC_VALUES, "last_written".as_bytes(), path.as_ref())?;
            write_txn.put(ACCOUNT_FLATKEYVALUE, path.as_ref(), &node.value)?;
            ctr += 1;
            if ctr > 10_000 {
                write_txn.commit()?;
                write_txn = backend.begin_write()?;
                *last_computed_fkv
                    .write()
                    .map_err(|_| StoreError::LockError)? = path.as_ref().to_vec();
                ctr = 0;
            }

            let mut iter_inner = Trie::open(
                Box::new(BackendTrieDB::new_for_account_storage_with_view(
                    backend.clone(),
                    read_tx.clone(),
                    account_hash,
                    path.as_ref().to_vec(),
                )?),
                account_state.storage_root,
            )
            .into_iter();
            if last_written_storage > Nibbles::default() {
                iter_inner.advance(last_written_storage.to_bytes())?;
                last_written_storage = Nibbles::default();
            }
            iter_inner.try_for_each(|(path, node)| -> Result<(), StoreError> {
                let Node::Leaf(node) = node else {
                    return Ok(());
                };
                let key = mpt_apply_prefix(Some(account_hash), path.into_vec());
                write_txn.put(MISC_VALUES, "last_written".as_bytes(), &key)?;
                write_txn.put(STORAGE_FLATKEYVALUE, &key, &node.value)?;
                ctr += 1;
                if ctr > 10_000 {
                    write_txn.commit()?;
                    write_txn = backend.begin_write()?;
                    *last_computed_fkv
                        .write()
                        .map_err(|_| StoreError::LockError)? = key;
                    ctr = 0;
                }
                fkv_check_for_stop_msg(control_rx)?;
                Ok(())
            })?;
            fkv_check_for_stop_msg(control_rx)?;
            Ok(())
        });
        match res {
            Err(StoreError::PivotChanged) => {
                match control_rx.recv() {
                    Ok(FKVGeneratorControlMessage::Continue) => {}
                    Ok(FKVGeneratorControlMessage::Stop) => {
                        return Err(StoreError::Custom("Unexpected Stop message".to_string()));
                    }
                    // If the channel was closed, we stop generation prematurely
                    Err(std::sync::mpsc::RecvError) => {
                        info!("Store closed, stopping FlatKeyValue generation.");
                        return Ok(());
                    }
                }
            }
            Err(err) => return Err(err),
            Ok(()) => {
                write_txn.put(MISC_VALUES, "last_written".as_bytes(), &[0xff])?;
                write_txn.commit()?;
                *last_computed_fkv
                    .write()
                    .map_err(|_| StoreError::LockError)? = vec![0xff; 131];
                info!("FlatKeyValue generation finished.");
                return Ok(());
            }
        };
    }
}

fn fkv_check_for_stop_msg(
    control_rx: &std::sync::mpsc::Receiver<FKVGeneratorControlMessage>,
) -> Result<(), StoreError> {
    match control_rx.try_recv() {
        Ok(FKVGeneratorControlMessage::Stop) | Err(TryRecvError::Disconnected) => {
            return Err(StoreError::PivotChanged);
        }
        Ok(FKVGeneratorControlMessage::Continue) => {
            return Err(StoreError::Custom(
                "Unexpected Continue message".to_string(),
            ));
        }
        Err(TryRecvError::Empty) => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// MptTrieWrapper — MPT-specific TrieDB adapter for TrieLayerCache
// ---------------------------------------------------------------------------

/// [`ethrex_trie::TrieDB`] adapter that checks in-memory diff-layers ([`crate::layering::TrieLayerCache`]) first,
/// falling back to the on-disk trie only for keys not found in any layer.
///
/// Used by the EVM during block execution: reads see the latest uncommitted state without
/// waiting for a disk flush. Keys are MPT-nibble bytes as written by `build_mpt_cache_layer`.
pub(crate) struct MptTrieWrapper {
    pub state_root: H256,
    pub inner: Arc<crate::layering::TrieLayerCache>,
    pub db: Box<dyn ethrex_trie::TrieDB>,
    /// Pre-computed prefix nibbles for storage tries.
    /// For state tries this is None; for storage tries this is
    /// `Nibbles::from_bytes(address.as_bytes()).append_new(17)`.
    prefix_nibbles: Option<Nibbles>,
}

impl MptTrieWrapper {
    pub fn new(
        state_root: H256,
        inner: Arc<crate::layering::TrieLayerCache>,
        db: Box<dyn ethrex_trie::TrieDB>,
        prefix: Option<H256>,
    ) -> Self {
        let prefix_nibbles = prefix.map(|p| Nibbles::from_bytes(p.as_bytes()).append_new(17));
        Self {
            state_root,
            inner,
            db,
            prefix_nibbles,
        }
    }
}

impl ethrex_trie::TrieDB for MptTrieWrapper {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        // Apply the storage-trie prefix here; the underlying TrieDB is always for the state trie.
        let key = match &self.prefix_nibbles {
            Some(prefix) => prefix.concat(&key),
            None => key,
        };
        self.db.flatkeyvalue_computed(key)
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = match &self.prefix_nibbles {
            Some(prefix) => prefix.concat(&key),
            None => key,
        };
        if let Some(value) = self.inner.get(self.state_root, key.as_ref()) {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!("MptTrieWrapper is read-only; put_batch should not be called");
    }
}

// ---------------------------------------------------------------------------
// mpt_apply_prefix — MPT-specific key prefixing for the flat KV namespace
// ---------------------------------------------------------------------------

/// Prepends an account address prefix (with an invalid nibble `17` as separator) to a
/// trie path (raw nibble data), distinguishing storage trie entries from state trie
/// entries in the flat key-value namespace. Returns the path unchanged if `prefix` is
/// `None` (state trie).
///
/// This is MPT-specific. Other backends must use their own key encoding.
pub(crate) fn mpt_apply_prefix(prefix: Option<H256>, path: Vec<u8>) -> Vec<u8> {
    match prefix {
        Some(prefix) => {
            let prefix_nibbles = Nibbles::from_bytes(prefix.as_bytes());
            let mut result = Vec::with_capacity(prefix_nibbles.len() + 1 + path.len());
            result.extend_from_slice(prefix_nibbles.as_ref());
            result.push(17);
            result.extend_from_slice(&path);
            result
        }
        None => path,
    }
}
