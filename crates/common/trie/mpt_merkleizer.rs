//! 16-shard parallel MPT merkleizer.
//!
//! Extracts the merkleization logic from `blockchain.rs` into a standalone
//! struct that can be constructed and driven through the `StateBackend` enum
//! in `ethrex-storage`.

use std::collections::hash_map::Entry;
use std::sync::{
    Arc, LazyLock,
    mpsc::{Sender, channel},
};

use crossbeam::channel as cb;
use crossbeam::select;

use ethereum_types::{H256, U256};
use ethrex_common::{
    Address,
    types::{AccountInfo, AccountState, AccountUpdate, Code},
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{constants::RLP_NULL, decode::RLPDecode, encode::RLPEncode};
use ethrex_state_backend::{MerkleOutput, NodeUpdates, StateError};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    EMPTY_TRIE_HASH, Nibbles, Trie, TrieError, TrieNode,
    node::{BranchNode, ExtensionNode, LeafNode, Node, NodeRef},
};

// ---------------------------------------------------------------------------
// StorageTrieOpener trait (MPT-specific, not in shared traits)
// ---------------------------------------------------------------------------

/// Opens a storage trie for a given account. Implemented in `ethrex-storage`
/// by closures that capture the `Store` and parent state root.
pub trait StorageTrieOpener: Send + Sync {
    fn open(&self, account_hash: H256, storage_root: H256) -> Result<Trie, TrieError>;
}

// ---------------------------------------------------------------------------
// Background drop thread (avoids recursive deallocation on hot path)
// ---------------------------------------------------------------------------

static DROP_SENDER: LazyLock<Sender<Box<dyn Send>>> = LazyLock::new(|| {
    let (tx, rx) = channel::<Box<dyn Send>>();
    std::thread::Builder::new()
        .name("drop_thread".to_string())
        .spawn(move || for _ in rx {})
        .expect("failed to spawn drop thread");
    tx
});

// ---------------------------------------------------------------------------
// Internal message types
// ---------------------------------------------------------------------------

enum WorkerRequest {
    ProcessAccount {
        prefix: H256,
        info: Option<AccountInfo>,
        storage: FxHashMap<H256, U256>,
        removed: bool,
        removed_storage: bool,
    },
    FinishRouting,
    MerklizeAccounts {
        accounts: Vec<H256>,
    },
    CollectState {
        tx: Sender<CollectedStateMsg>,
    },
    MerklizeStorage {
        prefix: H256,
        key: H256,
        value: U256,
        storage_root: H256,
    },
    DeleteStorage(H256),
    RoutingDone {
        from: u8,
    },
    StorageShard {
        prefix: H256,
        index: u8,
        subroot: Box<BranchNode>,
        nodes: Vec<TrieNode>,
    },
}

struct CollectedStateMsg {
    index: u8,
    subroot: Box<BranchNode>,
    state_nodes: Vec<TrieNode>,
    storage_nodes: Vec<(H256, Vec<TrieNode>)>,
}

#[derive(Default)]
struct PreMerkelizedAccountState {
    storage_root: Option<Box<BranchNode>>,
    nodes: Vec<TrieNode>,
}

struct BalStateWorkItem {
    hashed_address: H256,
    info: Option<AccountInfo>,
    removed: bool,
    storage_root: Option<H256>,
}

// ---------------------------------------------------------------------------
// MptMerkleizer
// ---------------------------------------------------------------------------

/// Streaming 16-shard MPT merkleizer.
///
/// Created via [`MptMerkleizer::new`] (standard) or [`MptMerkleizer::new_bal`]
/// (BAL-optimized). Call [`feed_updates`](MptMerkleizer::feed_updates) one or
/// more times, then [`finalize`](MptMerkleizer::finalize) to get the
/// [`MerkleOutput`].
pub struct MptMerkleizer {
    workers_tx: Vec<cb::Sender<WorkerRequest>>,
    /// Receives the watcher result (None = no error, Some(e) = worker error).
    watcher_rx: Option<cb::Receiver<Option<StateError>>>,
    code_updates: Vec<(H256, Code)>,
    hashed_address_cache: FxHashMap<Address, H256>,
    has_storage: FxHashSet<H256>,
    accumulate_for_witness: bool,
    accumulator: Option<FxHashMap<Address, AccountUpdate>>,
    // State trie + storage trie openers, retained for finalize.
    state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
    storage_trie_opener: Arc<dyn StorageTrieOpener>,
    // BAL-specific: when Some, feed_updates accumulates everything for finalize_bal.
    bal_all_updates: Option<FxHashMap<Address, AccountUpdate>>,
    // Rayon thread pool used by both new() and finalize_bal().
    pool: Arc<rayon::ThreadPool>,
}

impl MptMerkleizer {
    /// Create a standard (streaming) merkleizer that runs 16 shard workers on
    /// the provided `rayon::ThreadPool`, avoiding per-block OS thread creation.
    /// `_parent_state_root` is reserved for non-MPT backends; MPT captures
    /// the root inside the opener closures.
    pub fn new(
        _parent_state_root: H256,
        precompute_witnesses: bool,
        state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
        storage_trie_opener: Arc<dyn StorageTrieOpener>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        let mut workers_tx = Vec::with_capacity(16);
        let mut workers_rx = Vec::with_capacity(16);
        for _ in 0..16 {
            let (tx, rx) = cb::unbounded();
            workers_tx.push(tx);
            workers_rx.push(rx);
        }

        let (shutdown_tx, shutdown_rx) = cb::bounded::<()>(0);
        let (done_tx, done_rx) = cb::unbounded::<Result<(), StateError>>();

        for (i, rx) in workers_rx.into_iter().enumerate() {
            let all_senders = workers_tx.clone();
            let shutdown_rx = shutdown_rx.clone();
            let done_tx = done_tx.clone();
            let opener = Arc::clone(&state_trie_opener);
            let st_opener = Arc::clone(&storage_trie_opener);

            pool.spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_subtrie(opener, st_opener, rx, i as u8, all_senders, shutdown_rx)
                }));
                let result = match result {
                    Ok(r) => r,
                    Err(_) => Err(StateError::Other(format!("shard worker {i} panicked"))),
                };
                let _ = done_tx.send(result);
            });
        }
        drop(done_tx);
        drop(shutdown_rx);

        let (watcher_tx, watcher_rx) = cb::bounded::<Option<StateError>>(1);
        pool.spawn(move || {
            let _shutdown = shutdown_tx;
            let mut error: Option<StateError> = None;
            for result in done_rx {
                if let Err(e) = result {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
            }
            let _ = watcher_tx.send(error);
        });

        Ok(Self {
            workers_tx,
            watcher_rx: Some(watcher_rx),
            code_updates: Vec::new(),
            hashed_address_cache: FxHashMap::default(),
            has_storage: FxHashSet::default(),
            accumulate_for_witness: precompute_witnesses,
            accumulator: if precompute_witnesses {
                Some(FxHashMap::default())
            } else {
                None
            },
            state_trie_opener,
            storage_trie_opener,
            bal_all_updates: None,
            pool,
        })
    }

    /// Create a BAL-optimized merkleizer. Updates are accumulated in
    /// `feed_updates` and processed all at once in `finalize`.
    pub fn new_bal(
        _parent_state_root: H256,
        precompute_witnesses: bool,
        state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
        storage_trie_opener: Arc<dyn StorageTrieOpener>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        Ok(Self {
            workers_tx: Vec::new(),
            watcher_rx: None,
            code_updates: Vec::new(),
            hashed_address_cache: FxHashMap::default(),
            has_storage: FxHashSet::default(),
            accumulate_for_witness: precompute_witnesses,
            accumulator: if precompute_witnesses {
                Some(FxHashMap::default())
            } else {
                None
            },
            state_trie_opener,
            storage_trie_opener,
            bal_all_updates: Some(FxHashMap::default()),
            pool,
        })
    }

    /// Route a batch of account updates to shard workers.
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        if let Some(bal) = self.bal_all_updates.as_mut() {
            // BAL: just accumulate, actual work happens in finalize
            for update in updates {
                merge_update(bal, update);
            }
            return Ok(());
        }

        // Standard path: route to workers
        for update in updates {
            // Accumulate for witness generation before destructuring
            if let Some(acc) = &mut self.accumulator {
                merge_update(acc, update.clone());
            }

            let hashed_address = *self
                .hashed_address_cache
                .entry(update.address)
                .or_insert_with(|| keccak(update.address));

            let (info, code, storage) = if update.removed {
                (Some(Default::default()), None, Default::default())
            } else {
                (update.info, update.code, update.added_storage)
            };

            if let Some(ref info) = info
                && let Some(code) = code
            {
                self.code_updates.push((info.code_hash, code));
            }

            if update.removed || update.removed_storage || !storage.is_empty() {
                self.has_storage.insert(hashed_address);
            }

            let bucket = hashed_address.as_fixed_bytes()[0] >> 4;
            self.workers_tx[bucket as usize]
                .send(WorkerRequest::ProcessAccount {
                    prefix: hashed_address,
                    info,
                    storage,
                    removed: update.removed,
                    removed_storage: update.removed_storage,
                })
                .map_err(|e| StateError::Other(format!("send error: {e}")))?;
        }
        Ok(())
    }

    /// Finalize merkleization: collect all shard results and assemble root.
    pub fn finalize(self) -> Result<MerkleOutput, StateError> {
        if self.bal_all_updates.is_some() {
            return self.finalize_bal();
        }
        self.finalize_standard()
    }

    fn finalize_standard(mut self) -> Result<MerkleOutput, StateError> {
        // Signal workers
        for tx in &self.workers_tx {
            tx.send(WorkerRequest::FinishRouting)
                .map_err(|e| StateError::Other(format!("send error: {e}")))?;
        }

        // Send MerklizeAccounts for no-storage accounts
        let mut early_batches: [Vec<H256>; 16] = Default::default();
        for hashed_account in self.hashed_address_cache.values() {
            if !self.has_storage.contains(hashed_account) {
                let bucket = hashed_account.as_fixed_bytes()[0] >> 4;
                early_batches[bucket as usize].push(*hashed_account);
            }
        }
        for (i, batch) in early_batches.into_iter().enumerate() {
            if !batch.is_empty() {
                self.workers_tx[i]
                    .send(WorkerRequest::MerklizeAccounts { accounts: batch })
                    .map_err(|e| StateError::Other(format!("send error: {e}")))?;
            }
        }

        // Collect state
        let mut storage_updates: Vec<(H256, Vec<TrieNode>)> = Vec::new();
        let (gatherer_tx, gatherer_rx) = channel();
        for tx in &self.workers_tx {
            tx.send(WorkerRequest::CollectState {
                tx: gatherer_tx.clone(),
            })
            .map_err(|e| StateError::Other(format!("send error: {e}")))?;
        }
        drop(gatherer_tx);
        self.workers_tx.clear();

        let mut root = BranchNode::default();
        let mut state_updates: Vec<TrieNode> = Vec::new();
        for CollectedStateMsg {
            index,
            subroot,
            state_nodes,
            storage_nodes,
        } in gatherer_rx
        {
            storage_updates.extend(storage_nodes);
            state_updates.extend(state_nodes);
            root.choices[index as usize] = subroot.choices[index as usize].clone();
        }

        // Collapse and finalize root
        let opener = &self.state_trie_opener;
        let collapsed =
            collapse_root_node(&|| opener(), root).map_err(|e| StateError::Trie(e.to_string()))?;

        let state_trie_hash = if let Some(root) = collapsed {
            let mut root = NodeRef::from(root);
            let hash = root.commit(Nibbles::default(), &mut state_updates, &NativeCrypto);
            let _ = DROP_SENDER.send(Box::new(root));
            hash.finalize(&NativeCrypto)
        } else {
            state_updates.push((Nibbles::default(), vec![RLP_NULL]));
            *EMPTY_TRIE_HASH
        };

        // Check watcher for worker errors
        if let Some(rx) = self.watcher_rx.take()
            && let Ok(Some(err)) = rx.recv()
        {
            return Err(err);
        }

        let accumulated_updates = self
            .accumulator
            .take()
            .map(|acc| acc.into_values().collect());

        Ok(MerkleOutput {
            root: state_trie_hash,
            node_updates: NodeUpdates::Mpt {
                state_updates: state_updates
                    .into_iter()
                    .map(|(nib, rlp)| (nib.into_vec(), rlp))
                    .collect(),
                storage_updates: storage_updates
                    .into_iter()
                    .map(|(addr, nodes)| {
                        (
                            addr,
                            nodes
                                .into_iter()
                                .map(|(nib, rlp)| (nib.into_vec(), rlp))
                                .collect(),
                        )
                    })
                    .collect(),
            },
            code_updates: std::mem::take(&mut self.code_updates),
            accumulated_updates,
        })
    }

    fn finalize_bal(mut self) -> Result<MerkleOutput, StateError> {
        const NUM_WORKERS: usize = 16;

        let all_updates = self
            .bal_all_updates
            .take()
            .ok_or_else(|| StateError::Other("finalize_bal called without BAL mode".into()))?;

        let accumulated_updates = if self.accumulate_for_witness {
            Some(all_updates.values().cloned().collect::<Vec<_>>())
        } else {
            None
        };

        // Extract code updates and build work items
        let mut accounts: Vec<(H256, AccountUpdate)> = Vec::with_capacity(all_updates.len());
        for (addr, update) in all_updates {
            let hashed = keccak(addr);
            if let Some(info) = &update.info
                && let Some(code) = &update.code
            {
                self.code_updates.push((info.code_hash, code.clone()));
            }
            accounts.push((hashed, update));
        }

        // Stage B: Parallel per-account storage root computation
        let mut work_indices: Vec<(usize, usize)> = accounts
            .iter()
            .enumerate()
            .map(|(i, (_, update))| {
                let weight =
                    if update.removed || update.removed_storage || !update.added_storage.is_empty()
                    {
                        1.max(update.added_storage.len())
                    } else {
                        0
                    };
                (i, weight)
            })
            .collect();
        work_indices.sort_unstable_by(|a, b| b.1.cmp(&a.1));

        let mut bins: Vec<Vec<usize>> = (0..NUM_WORKERS).map(|_| Vec::new()).collect();
        let mut bin_weights: Vec<usize> = vec![0; NUM_WORKERS];
        for (idx, weight) in work_indices {
            let min_bin = bin_weights
                .iter()
                .enumerate()
                .min_by_key(|(_, w)| **w)
                .ok_or_else(|| StateError::Other("empty bin_weights".into()))?
                .0;
            bins[min_bin].push(idx);
            bin_weights[min_bin] += weight;
        }

        let mut storage_roots: Vec<Option<H256>> = vec![None; accounts.len()];
        let mut storage_updates: Vec<(H256, Vec<TrieNode>)> = Vec::new();

        let st_opener = &self.state_trie_opener;
        let sto_opener = &self.storage_trie_opener;

        // Use a channel to collect worker results from the pool scope.
        let (storage_result_tx, storage_result_rx) =
            cb::unbounded::<Result<Vec<(usize, H256, Vec<TrieNode>)>, StateError>>();
        self.pool.scope(|s| {
            let accounts_ref = &accounts;
            for (_worker_id, bin) in bins.into_iter().enumerate() {
                if bin.is_empty() {
                    continue;
                }
                let tx = storage_result_tx.clone();
                s.spawn(move |_| {
                    let result = (|| -> Result<Vec<(usize, H256, Vec<TrieNode>)>, StateError> {
                        let mut results = Vec::new();
                        let state_trie =
                            st_opener().map_err(|e| StateError::Trie(e.to_string()))?;
                        for idx in bin {
                            let (hashed_address, update) = &accounts_ref[idx];
                            let has_storage_changes = update.removed
                                || update.removed_storage
                                || !update.added_storage.is_empty();
                            if !has_storage_changes {
                                continue;
                            }

                            if update.removed {
                                results.push((
                                    idx,
                                    *EMPTY_TRIE_HASH,
                                    vec![(Nibbles::default(), vec![RLP_NULL])],
                                ));
                                continue;
                            }

                            let mut trie = if update.removed_storage {
                                Trie::new_temp()
                            } else {
                                let storage_root = match state_trie
                                    .get(hashed_address.as_bytes())
                                    .map_err(|e| {
                                    StateError::Trie(e.to_string())
                                })? {
                                    Some(rlp) => {
                                        AccountState::decode(&rlp)
                                            .map_err(|e| StateError::Trie(e.to_string()))?
                                            .storage_root
                                    }
                                    None => *EMPTY_TRIE_HASH,
                                };
                                sto_opener
                                    .open(*hashed_address, storage_root)
                                    .map_err(|e| StateError::Trie(e.to_string()))?
                            };

                            for (key, value) in &update.added_storage {
                                let hashed_key = keccak(key);
                                if value.is_zero() {
                                    trie.remove(hashed_key.as_bytes())
                                        .map_err(|e| StateError::Trie(e.to_string()))?;
                                } else {
                                    trie.insert(
                                        hashed_key.as_bytes().to_vec(),
                                        value.encode_to_vec(),
                                    )
                                    .map_err(|e| StateError::Trie(e.to_string()))?;
                                }
                            }

                            let (root_hash, nodes) =
                                trie.collect_changes_since_last_hash(&NativeCrypto);
                            results.push((idx, root_hash, nodes));
                        }
                        Ok(results)
                    })();
                    let _ = tx.send(result);
                });
            }
            drop(storage_result_tx);
        });
        for worker_result in storage_result_rx {
            let results = worker_result
                .map_err(|e| StateError::Other(format!("bal storage worker error: {e}")))?;
            for (idx, root_hash, nodes) in results {
                storage_roots[idx] = Some(root_hash);
                storage_updates.push((accounts[idx].0, nodes));
            }
        }

        // Stage C: State trie update via 16 shard workers
        let mut shards: Vec<Vec<BalStateWorkItem>> = (0..NUM_WORKERS).map(|_| Vec::new()).collect();
        for (idx, (hashed_address, update)) in accounts.iter().enumerate() {
            let bucket = (hashed_address.as_fixed_bytes()[0] >> 4) as usize;
            shards[bucket].push(BalStateWorkItem {
                hashed_address: *hashed_address,
                info: update.info.clone(),
                removed: update.removed,
                storage_root: storage_roots[idx],
            });
        }

        let mut root = BranchNode::default();
        let mut state_updates: Vec<TrieNode> = Vec::new();

        let (state_shard_tx, state_shard_rx) =
            cb::unbounded::<(usize, Result<(Box<BranchNode>, Vec<TrieNode>), StateError>)>();
        self.pool.scope(|s| {
            for (index, shard_items) in shards.into_iter().enumerate() {
                let tx = state_shard_tx.clone();
                s.spawn(move |_| {
                    let result = (|| -> Result<(Box<BranchNode>, Vec<TrieNode>), StateError> {
                        let mut state_trie =
                            st_opener().map_err(|e| StateError::Trie(e.to_string()))?;

                        for item in &shard_items {
                            let path = item.hashed_address.as_bytes();
                            let mut account_state = match state_trie
                                .get(path)
                                .map_err(|e| StateError::Trie(e.to_string()))?
                            {
                                Some(rlp) => {
                                    let state = AccountState::decode(&rlp)
                                        .map_err(|e| StateError::Trie(e.to_string()))?;
                                    state_trie
                                        .insert(path.to_vec(), rlp)
                                        .map_err(|e| StateError::Trie(e.to_string()))?;
                                    state
                                }
                                None => AccountState::default(),
                            };

                            if item.removed {
                                account_state = AccountState::default();
                            } else {
                                if let Some(ref info) = item.info {
                                    account_state.nonce = info.nonce;
                                    account_state.balance = info.balance;
                                    account_state.code_hash = info.code_hash;
                                }
                                if let Some(storage_root) = item.storage_root {
                                    account_state.storage_root = storage_root;
                                }
                            }

                            if account_state != AccountState::default() {
                                state_trie
                                    .insert(path.to_vec(), account_state.encode_to_vec())
                                    .map_err(|e| StateError::Trie(e.to_string()))?;
                            } else {
                                state_trie
                                    .remove(path)
                                    .map_err(|e| StateError::Trie(e.to_string()))?;
                            }
                        }

                        collect_trie(index as u8, state_trie)
                            .map_err(|e| StateError::Trie(e.to_string()))
                    })();
                    let _ = tx.send((index, result));
                });
            }
            drop(state_shard_tx);
        });
        for (i, result) in state_shard_rx {
            let (subroot, state_nodes) =
                result.map_err(|e| StateError::Other(format!("bal state shard error: {e}")))?;
            state_updates.extend(state_nodes);
            root.choices[i] = subroot.choices[i].clone();
        }

        // Stage D: Finalize root
        let collapsed = collapse_root_node(&|| st_opener(), root)
            .map_err(|e| StateError::Trie(e.to_string()))?;

        let state_trie_hash = if let Some(root) = collapsed {
            let mut root = NodeRef::from(root);
            let hash = root.commit(Nibbles::default(), &mut state_updates, &NativeCrypto);
            let _ = DROP_SENDER.send(Box::new(root));
            hash.finalize(&NativeCrypto)
        } else {
            state_updates.push((Nibbles::default(), vec![RLP_NULL]));
            *EMPTY_TRIE_HASH
        };

        Ok(MerkleOutput {
            root: state_trie_hash,
            node_updates: NodeUpdates::Mpt {
                state_updates: state_updates
                    .into_iter()
                    .map(|(nib, rlp)| (nib.into_vec(), rlp))
                    .collect(),
                storage_updates: storage_updates
                    .into_iter()
                    .map(|(addr, nodes)| {
                        (
                            addr,
                            nodes
                                .into_iter()
                                .map(|(nib, rlp)| (nib.into_vec(), rlp))
                                .collect(),
                        )
                    })
                    .collect(),
            },
            code_updates: std::mem::take(&mut self.code_updates),
            accumulated_updates,
        })
    }
}

impl Drop for MptMerkleizer {
    fn drop(&mut self) {
        // Drop all senders to signal workers to stop.
        // Pool tasks will complete when the channels close.
        self.workers_tx.clear();
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn handle_subtrie(
    state_trie_opener: Arc<dyn Fn() -> Result<Trie, TrieError> + Send + Sync>,
    storage_trie_opener: Arc<dyn StorageTrieOpener>,
    rx: cb::Receiver<WorkerRequest>,
    index: u8,
    worker_senders: Vec<cb::Sender<WorkerRequest>>,
    shutdown_rx: cb::Receiver<()>,
) -> Result<(), StateError> {
    let mut state_trie = state_trie_opener().map_err(|e| StateError::Trie(e.to_string()))?;
    let mut storage_nodes: Vec<(H256, Vec<TrieNode>)> = vec![];
    let mut accounts: FxHashMap<H256, AccountState> = Default::default();
    let mut expected_shards: FxHashMap<H256, u16> = Default::default();
    let mut storage_state: FxHashMap<H256, PreMerkelizedAccountState> = Default::default();
    let mut received_shards: FxHashMap<H256, u16> = Default::default();
    let mut pending_storage_accounts: usize = 0;
    let mut pending_collect_tx: Option<Sender<CollectedStateMsg>> = None;
    let mut pre_collected_state: Vec<TrieNode> = vec![];
    let mut storage_tries: FxHashMap<H256, Trie> = Default::default();
    let mut pre_collected_storage: FxHashMap<H256, Vec<TrieNode>> = Default::default();

    let mut worker_senders: Option<Vec<cb::Sender<WorkerRequest>>> = Some(worker_senders);
    let mut dirty = false;
    let mut collecting_storages = false;
    let mut routing_complete = false;
    let mut routing_done_mask: u16 = 0;
    let mut storage_to_collect: Vec<(H256, Trie)> = vec![];

    loop {
        if collecting_storages {
            if let Some((prefix, trie)) = storage_to_collect.pop() {
                let senders = worker_senders
                    .as_ref()
                    .ok_or_else(|| StateError::Other("collecting after senders dropped".into()))?;
                let (root, mut nodes) =
                    collect_trie(index, trie).map_err(|e| StateError::Trie(e.to_string()))?;
                if let Some(mut pre_nodes) = pre_collected_storage.remove(&prefix) {
                    pre_nodes.extend(nodes);
                    nodes = pre_nodes;
                }
                let bucket = prefix.as_fixed_bytes()[0] >> 4;
                senders[bucket as usize]
                    .send(WorkerRequest::StorageShard {
                        prefix,
                        index,
                        subroot: root,
                        nodes,
                    })
                    .map_err(|e| StateError::Other(format!("send error: {e}")))?;
            } else {
                worker_senders = None;
                collecting_storages = false;
                if pending_storage_accounts == 0
                    && let Some(tx) = pending_collect_tx.take()
                {
                    collect_and_send(
                        index,
                        &mut state_trie,
                        &mut pre_collected_state,
                        &mut storage_nodes,
                        tx,
                    )?;
                    break;
                }
            }
        }

        let msg = if collecting_storages || dirty {
            match rx.try_recv() {
                Ok(msg) => msg,
                Err(cb::TryRecvError::Disconnected) => break,
                Err(cb::TryRecvError::Empty) => {
                    if matches!(shutdown_rx.try_recv(), Err(cb::TryRecvError::Disconnected)) {
                        return Err(StateError::Other("shard worker shutdown".into()));
                    }
                    if dirty {
                        let mut nodes = state_trie.commit_without_storing(&NativeCrypto);
                        nodes.retain(|(nib, _)| nib.as_ref().first() == Some(&index));
                        pre_collected_state.extend(nodes);
                        if !collecting_storages {
                            for (prefix, trie) in storage_tries.iter_mut() {
                                let mut nodes = trie.commit_without_storing(&NativeCrypto);
                                nodes.retain(|(nib, _)| nib.as_ref().first() == Some(&index));
                                if !nodes.is_empty() {
                                    pre_collected_storage
                                        .entry(*prefix)
                                        .or_default()
                                        .extend(nodes);
                                }
                            }
                        }
                        dirty = false;
                    }
                    continue;
                }
            }
        } else {
            select! {
                recv(rx) -> msg => match msg {
                    Ok(msg) => msg,
                    Err(_) => break,
                },
                recv(shutdown_rx) -> _ => {
                    return Err(StateError::Other("shard worker shutdown".into()));
                }
            }
        };

        match msg {
            WorkerRequest::ProcessAccount {
                prefix,
                info,
                storage: account_storage,
                removed,
                removed_storage,
            } => {
                let senders = worker_senders.as_ref().ok_or_else(|| {
                    StateError::Other("ProcessAccount after collection started".into())
                })?;

                // Load account state if not already cached
                if let Entry::Vacant(vacant) = accounts.entry(prefix) {
                    let account_state = match state_trie
                        .get(prefix.as_bytes())
                        .map_err(|e| StateError::Trie(e.to_string()))?
                    {
                        Some(rlp) => {
                            let state = AccountState::decode(&rlp)
                                .map_err(|e| StateError::Trie(e.to_string()))?;
                            state_trie
                                .insert(prefix.as_bytes().to_vec(), rlp)
                                .map_err(|e| StateError::Trie(e.to_string()))?;
                            state
                        }
                        None => AccountState::default(),
                    };
                    vacant.insert(account_state);
                }

                if let Some(info) = info {
                    let acct = accounts
                        .get_mut(&prefix)
                        .ok_or_else(|| StateError::Other("account not loaded".into()))?;
                    acct.nonce = info.nonce;
                    acct.balance = info.balance;
                    acct.code_hash = info.code_hash;
                    let path = prefix.as_bytes();
                    if *acct != AccountState::default() {
                        state_trie
                            .insert(path.to_vec(), acct.encode_to_vec())
                            .map_err(|e| StateError::Trie(e.to_string()))?;
                    } else {
                        state_trie
                            .remove(path)
                            .map_err(|e| StateError::Trie(e.to_string()))?;
                    }
                }

                if removed || removed_storage {
                    pre_collected_storage.remove(&prefix);
                    storage_tries.insert(prefix, Trie::new_temp());
                    for (i, tx) in senders.iter().enumerate() {
                        if i as u8 != index {
                            tx.send(WorkerRequest::DeleteStorage(prefix))
                                .map_err(|e| StateError::Other(format!("send error: {e}")))?;
                        }
                    }
                    accounts
                        .get_mut(&prefix)
                        .ok_or_else(|| StateError::Other("account not loaded".into()))?
                        .storage_root = *EMPTY_TRIE_HASH;
                    if expected_shards.insert(prefix, 0xFFFF).is_none() {
                        pending_storage_accounts += 1;
                    }
                    if removed {
                        dirty = true;
                        continue;
                    }
                }

                if !account_storage.is_empty() {
                    let storage_root = accounts
                        .get(&prefix)
                        .map(|a| a.storage_root)
                        .unwrap_or(*EMPTY_TRIE_HASH);

                    let is_new = !expected_shards.contains_key(&prefix);
                    for (key, value) in account_storage {
                        let hashed_key = keccak(key);
                        let bucket = hashed_key.as_fixed_bytes()[0] >> 4;
                        *expected_shards.entry(prefix).or_insert(0u16) |= 1 << bucket;
                        if bucket == index {
                            let trie = get_or_open_storage_trie(
                                &mut storage_tries,
                                storage_trie_opener.as_ref(),
                                prefix,
                                storage_root,
                            )?;
                            if value.is_zero() {
                                trie.remove(hashed_key.as_bytes())
                                    .map_err(|e| StateError::Trie(e.to_string()))?;
                            } else {
                                trie.insert(hashed_key.as_bytes().to_vec(), value.encode_to_vec())
                                    .map_err(|e| StateError::Trie(e.to_string()))?;
                            }
                        } else {
                            senders[bucket as usize]
                                .send(WorkerRequest::MerklizeStorage {
                                    prefix,
                                    key: hashed_key,
                                    value,
                                    storage_root,
                                })
                                .map_err(|e| StateError::Other(format!("send error: {e}")))?;
                        }
                    }
                    if is_new {
                        pending_storage_accounts += 1;
                    }
                }
                dirty = true;
            }
            WorkerRequest::MerklizeStorage {
                prefix,
                key,
                value,
                storage_root,
            } => {
                let trie = get_or_open_storage_trie(
                    &mut storage_tries,
                    storage_trie_opener.as_ref(),
                    prefix,
                    storage_root,
                )?;
                if value.is_zero() {
                    trie.remove(key.as_bytes())
                        .map_err(|e| StateError::Trie(e.to_string()))?;
                } else {
                    trie.insert(key.as_bytes().to_vec(), value.encode_to_vec())
                        .map_err(|e| StateError::Trie(e.to_string()))?;
                }
                dirty = true;
            }
            WorkerRequest::DeleteStorage(prefix) => {
                pre_collected_storage.remove(&prefix);
                storage_tries.insert(prefix, Trie::new_temp());
                dirty = true;
            }
            WorkerRequest::FinishRouting => {
                let senders = worker_senders.as_ref().ok_or_else(|| {
                    StateError::Other("FinishRouting after senders dropped".into())
                })?;
                for i in 0..16u8 {
                    senders[i as usize]
                        .send(WorkerRequest::RoutingDone { from: index })
                        .map_err(|e| StateError::Other(format!("send error: {e}")))?;
                }
            }
            WorkerRequest::RoutingDone { from } => {
                routing_done_mask |= 1u16 << from;
                if routing_done_mask == 0xFFFF && !collecting_storages && !routing_complete {
                    collecting_storages = true;
                    routing_complete = true;
                    storage_to_collect = storage_tries.drain().collect();
                }
            }
            WorkerRequest::MerklizeAccounts { accounts: batch } => {
                for hashed_account in batch {
                    storage_nodes.push((hashed_account, vec![]));
                }
            }
            WorkerRequest::StorageShard {
                prefix,
                index: shard_index,
                mut subroot,
                nodes,
            } => {
                let state = storage_state.entry(prefix).or_default();
                match &mut state.storage_root {
                    Some(root) => {
                        root.choices[shard_index as usize] =
                            std::mem::take(&mut subroot.choices[shard_index as usize]);
                    }
                    rootptr => {
                        *rootptr = Some(subroot);
                    }
                }
                state.nodes.extend(nodes);

                let received = received_shards.entry(prefix).or_insert(0u16);
                *received |= 1 << shard_index;
                if *received == expected_shards.get(&prefix).copied().unwrap_or(0) {
                    let mut state = storage_state
                        .remove(&prefix)
                        .ok_or_else(|| StateError::Other("shard without state".into()))?;
                    let new_storage_root = if let Some(mut root) = state.storage_root {
                        root.choices.iter_mut().for_each(NodeRef::clear_hash);
                        let existing_storage_root = accounts
                            .get(&prefix)
                            .map(|a| a.storage_root)
                            .unwrap_or(*EMPTY_TRIE_HASH);
                        let collapsed = collapse_root_node(
                            &|| storage_trie_opener.open(prefix, existing_storage_root),
                            *root,
                        )
                        .map_err(|e| StateError::Trie(e.to_string()))?;
                        if let Some(root) = collapsed {
                            let mut root = NodeRef::from(root);
                            let hash =
                                root.commit(Nibbles::default(), &mut state.nodes, &NativeCrypto);
                            let _ = DROP_SENDER.send(Box::new(root));
                            hash.finalize(&NativeCrypto)
                        } else {
                            state.nodes.push((Nibbles::default(), vec![RLP_NULL]));
                            *EMPTY_TRIE_HASH
                        }
                    } else {
                        *EMPTY_TRIE_HASH
                    };
                    storage_nodes.push((prefix, state.nodes));

                    let old_state = accounts
                        .get_mut(&prefix)
                        .ok_or_else(|| StateError::Other("account not loaded".into()))?;
                    old_state.storage_root = new_storage_root;
                    let path = prefix.as_bytes();
                    if *old_state != AccountState::default() {
                        state_trie
                            .insert(path.to_vec(), old_state.encode_to_vec())
                            .map_err(|e| StateError::Trie(e.to_string()))?;
                    } else {
                        state_trie
                            .remove(path)
                            .map_err(|e| StateError::Trie(e.to_string()))?;
                    }

                    dirty = true;
                    pending_storage_accounts -= 1;
                    if pending_storage_accounts == 0
                        && !collecting_storages
                        && routing_complete
                        && let Some(tx) = pending_collect_tx.take()
                    {
                        collect_and_send(
                            index,
                            &mut state_trie,
                            &mut pre_collected_state,
                            &mut storage_nodes,
                            tx,
                        )?;
                        break;
                    }
                }
            }
            WorkerRequest::CollectState { tx } => {
                if pending_storage_accounts == 0 && !collecting_storages && routing_complete {
                    collect_and_send(
                        index,
                        &mut state_trie,
                        &mut pre_collected_state,
                        &mut storage_nodes,
                        tx,
                    )?;
                    break;
                }
                pending_collect_tx = Some(tx);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn merge_update(map: &mut FxHashMap<Address, AccountUpdate>, update: AccountUpdate) {
    match map.entry(update.address) {
        Entry::Vacant(e) => {
            e.insert(update);
        }
        Entry::Occupied(mut e) => {
            e.get_mut().merge(update);
        }
    }
}

fn get_or_open_storage_trie<'a>(
    storage_tries: &'a mut FxHashMap<H256, Trie>,
    opener: &dyn StorageTrieOpener,
    prefix: H256,
    storage_root: H256,
) -> Result<&'a mut Trie, StateError> {
    match storage_tries.entry(prefix) {
        Entry::Occupied(e) => Ok(e.into_mut()),
        Entry::Vacant(e) => Ok(e.insert(
            opener
                .open(prefix, storage_root)
                .map_err(|e| StateError::Trie(e.to_string()))?,
        )),
    }
}

fn collect_and_send(
    index: u8,
    state_trie: &mut Trie,
    pre_collected_state: &mut Vec<TrieNode>,
    storage_nodes: &mut Vec<(H256, Vec<TrieNode>)>,
    tx: Sender<CollectedStateMsg>,
) -> Result<(), StateError> {
    let (subroot, mut state_nodes) = collect_trie(index, std::mem::take(state_trie))
        .map_err(|e| StateError::Trie(e.to_string()))?;
    if !pre_collected_state.is_empty() {
        let mut pre = std::mem::take(pre_collected_state);
        pre.extend(state_nodes);
        state_nodes = pre;
    }
    tx.send(CollectedStateMsg {
        index,
        subroot,
        state_nodes,
        storage_nodes: std::mem::take(storage_nodes),
    })
    .map_err(|e| StateError::Other(format!("send error: {e}")))?;
    Ok(())
}

fn collect_trie(index: u8, mut trie: Trie) -> Result<(Box<BranchNode>, Vec<TrieNode>), TrieError> {
    let root = branchify(
        trie.root_node()?
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| Node::Branch(Box::default())),
    );
    trie.root = Node::Branch(root).into();
    let (_, mut nodes) = trie.collect_changes_since_last_hash(&NativeCrypto);
    nodes.retain(|(nib, _)| nib.as_ref().first() == Some(&index));

    let Some(Node::Branch(root)) = trie.root_node()?.map(Arc::unwrap_or_clone) else {
        return Err(TrieError::InvalidInput);
    };
    Ok((root, nodes))
}

fn branchify(node: Node) -> Box<BranchNode> {
    match node {
        Node::Branch(branch_node) => branch_node,
        Node::Extension(extension_node) => {
            let index = extension_node.prefix.as_ref()[0];
            let noderef = if extension_node.prefix.len() == 1 {
                extension_node.child
            } else {
                let prefix = extension_node.prefix.offset(1);
                let node = ExtensionNode::new(prefix, extension_node.child);
                NodeRef::from(Arc::new(node.into()))
            };
            let mut choices = BranchNode::EMPTY_CHOICES;
            choices[index as usize] = noderef;
            Box::new(BranchNode::new(choices))
        }
        Node::Leaf(leaf_node) => {
            let index = leaf_node.partial.as_ref()[0];
            let node = LeafNode::new(leaf_node.partial.offset(1), leaf_node.value);
            let mut choices = BranchNode::EMPTY_CHOICES;
            choices[index as usize] = NodeRef::from(Arc::new(node.into()));
            Box::new(BranchNode::new(choices))
        }
    }
}

fn collapse_root_node(
    load_trie: &dyn Fn() -> Result<Trie, TrieError>,
    root: BranchNode,
) -> Result<Option<Node>, TrieError> {
    let children: Vec<(usize, &NodeRef)> = root
        .choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| choice.is_valid())
        .take(2)
        .collect();
    if children.len() > 1 {
        return Ok(Some(Node::Branch(Box::from(root))));
    }
    let Some((choice, only_child)) = children.first() else {
        return Ok(None);
    };
    let only_child = Arc::unwrap_or_clone(match only_child {
        NodeRef::Node(node, _) => node.clone(),
        noderef @ NodeRef::Hash(_) => {
            let trie = load_trie()?;
            let Some(node) = noderef.get_node(trie.db(), Nibbles::from_hex(vec![*choice as u8]))?
            else {
                return Ok(None);
            };
            node
        }
    });
    Ok(Some(match only_child {
        Node::Branch(_) => {
            ExtensionNode::new(Nibbles::from_hex(vec![*choice as u8]), only_child.into()).into()
        }
        Node::Extension(mut extension_node) => {
            extension_node.prefix.prepend(*choice as u8);
            extension_node.into()
        }
        Node::Leaf(mut leaf) => {
            leaf.partial.prepend(*choice as u8);
            leaf.into()
        }
    }))
}
