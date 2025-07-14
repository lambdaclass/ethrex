use crate::{
    UpdateBatch,
    api::{KEEP_BLOCKS, StoreEngine},
    error::StoreError,
    store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS},
    store_db::codec::{
        account_info_log_entry::AccountInfoLogEntry,
        account_storage_log_entry::AccountStorageLogEntry, block_num_hash::BlockNumHash,
    },
};
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::Address;
use ethrex_common::types::{
    AccountInfo, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig,
    Index, Receipt, payload::PayloadBundle,
};
use ethrex_trie::{InMemoryTrieDB, Nibbles, NodeHash, Trie};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
    thread,
};
pub type NodeMap = Arc<Mutex<HashMap<NodeHash, Vec<u8>>>>;

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
struct StoreInner {
    chain_data: ChainData,
    block_numbers: HashMap<BlockHash, BlockNumber>,
    canonical_hashes: HashMap<BlockNumber, BlockHash>,
    bodies: HashMap<BlockHash, BlockBody>,
    headers: HashMap<BlockHash, BlockHeader>,
    // Maps code hashes to code
    account_codes: HashMap<H256, Bytes>,
    // Maps transaction hashes to their blocks (height+hash) and index within the blocks.
    transaction_locations: HashMap<H256, Vec<(BlockNumber, BlockHash, Index)>>,
    receipts: HashMap<BlockHash, HashMap<Index, Receipt>>,
    state_trie_nodes: NodeMap,
    // A storage trie for each hashed account address
    storage_trie_nodes: HashMap<H256, NodeMap>,
    // Stores local blocks by payload id
    payloads: HashMap<u64, PayloadBundle>,
    pending_blocks: HashMap<BlockHash, Block>,
    // Stores invalid blocks and their latest valid ancestor
    invalid_ancestors: HashMap<BlockHash, BlockHash>,
    // Stores current Snap Sate
    snap_state: SnapState,
    // Stores State trie leafs from the last downloaded tries
    state_snapshot: BTreeMap<H256, AccountState>,
    // Stores Storage trie leafs from the last downloaded tries
    storage_snapshot: HashMap<H256, BTreeMap<H256, U256>>,
    /// Stores current account info
    account_info: HashMap<Address, AccountInfo>,
    /// Stores current account storage
    account_storage: HashMap<(Address, H256), U256>,
    /// Current snapshot block number and hash
    current_snapshot_block: Option<BlockNumHash>,
    /// Account info write log table.
    /// The key maps to two blocks: first, and as seek key, the block corresponding to the final
    /// state after applying the log, and second, the parent of the first block in the range,
    /// that is, the state to which this log should be applied and the state we get back after
    /// rewinding these logs.
    account_state_logs: HashMap<BlockNumHash, Vec<(BlockNumHash, AccountInfoLogEntry)>>,
    /// Storage write log table.
    /// The key maps to two blocks: first, and as seek key, the block corresponding to the final
    /// state after applying the log, and second, the parent of the first block in the range,
    /// that is, the state to which this log should be applied and the state we get back after
    /// rewinding these logs.
    account_storage_logs: HashMap<BlockNumHash, Vec<(BlockNumHash, AccountStorageLogEntry)>>,
    /// State trie pruning log
    state_trie_pruning_log: BTreeMap<BlockNumHash, HashSet<[u8; 32]>>,
    /// Storage trie pruning log
    storage_trie_pruning_log: BTreeMap<BlockNumHash, HashSet<([u8; 32], NodeHash)>>,
    /// Reference counters for [`state_trie_nodes`](`StoreInner::state_trie_nodes`)
    /// Used to keep track of the number of times a node is referenced and avoid deleting it
    /// when it is still referenced by the state trie.
    /// This counter is incremented when a node is inserted into the state trie
    /// and decremented when a node is removed from the state trie. When the counter reaches 0,
    /// the node is deleted.
    state_trie_ref_counters: HashMap<NodeHash, u32>,
    /// Reference counters for [`storage_trie_nodes`](`StoreInner::storage_trie_nodes`)
    /// Used to keep track of the number of times a node is referenced and avoid deleting it
    /// when it is still referenced by the storage trie.
    /// This counter is incremented when a node is inserted into the storage trie
    /// and decremented when a node is removed from the storage trie. When the counter reaches 0,
    /// the node is deleted.
    storage_trie_ref_counters: HashMap<(H256, NodeHash), u32>,
}

#[derive(Default, Debug)]
struct ChainData {
    chain_config: Option<ChainConfig>,
    earliest_block_number: Option<BlockNumber>,
    finalized_block_number: Option<BlockNumber>,
    safe_block_number: Option<BlockNumber>,
    latest_block_number: Option<BlockNumber>,
    pending_block_number: Option<BlockNumber>,
}

// Keeps track of the state left by the latest snap attempt
#[derive(Default, Debug)]
pub struct SnapState {
    /// Latest downloaded block header's hash from a previously aborted sync
    header_download_checkpoint: Option<BlockHash>,
    /// Last downloaded key of the latest State Trie
    state_trie_key_checkpoint: Option<[H256; STATE_TRIE_SEGMENTS]>,
    /// Accounts which storage needs healing
    storage_heal_paths: Option<Vec<(H256, Vec<Nibbles>)>>,
    /// State trie Paths in need of healing
    state_heal_paths: Option<Vec<Nibbles>>,
    /// Storage tries waiting rebuild
    storage_trie_rebuild_pending: Option<Vec<(H256, H256)>>,
    // Latest root of the rebuilt state trie + the last inserted keys for each state trie segment
    state_trie_rebuild_checkpoint: Option<(H256, [H256; STATE_TRIE_SEGMENTS])>,
}

impl Store {
    pub fn new() -> Self {
        let store = Self::default();

        let store_for_pruning = store.clone();

        let _join = thread::Builder::new()
            .name("trie_pruner🗑️".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(std::time::Duration::from_secs(1));
                    #[allow(clippy::unwrap_used)]
                    store_for_pruning.prune_state_and_storage_log().unwrap();
                }
            });

        store
    }

    fn inner(&self) -> Result<MutexGuard<'_, StoreInner>, StoreError> {
        self.0.lock().map_err(|_| StoreError::LockError)
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        // Validation first - fail fast
        let (Some(first_block), Some(last_block)) =
            (update_batch.blocks.first(), update_batch.blocks.last())
        else {
            return Ok(());
        };

        let parent_block: BlockNumHash = (
            first_block.header.number - 1,
            first_block.header.parent_hash,
        )
            .into();
        let final_block: BlockNumHash = (last_block.header.number, last_block.hash()).into();

        // Preparar state trie updates
        let mut state_trie_updates = Vec::new();
        for (node_hash, node_data) in update_batch.account_updates {
            tracing::debug!(
                node_hash = hex::encode(node_hash),
                parent_block_number = parent_block.block_number,
                parent_block_hash = hex::encode(parent_block.block_hash),
                final_block_number = final_block.block_number,
                final_block_hash = hex::encode(final_block.block_hash),
                "[WRITING STATE TRIE NODE]",
            );
            state_trie_updates.push((node_hash, node_data));
        }

        let mut storage_updates_by_address: HashMap<H256, Vec<(NodeHash, Vec<u8>)>> =
            HashMap::new();
        let mut storage_invalidations_by_address: HashMap<H256, Vec<H256>> = HashMap::new();

        for (hashed_address, nodes, invalid_nodes) in update_batch.storage_updates {
            let mut prepared_nodes = Vec::new();
            for (node_hash, node_data) in nodes {
                tracing::debug!(
                    hashed_address = hex::encode(hashed_address.0),
                    node_hash = hex::encode(node_hash),
                    parent_block_number = parent_block.block_number,
                    parent_block_hash = hex::encode(parent_block.block_hash),
                    final_block_number = final_block.block_number,
                    final_block_hash = hex::encode(final_block.block_hash),
                    "[WRITING STORAGE TRIE NODE]",
                );
                prepared_nodes.push((node_hash, node_data));
            }
            storage_updates_by_address.insert(hashed_address, prepared_nodes);

            storage_invalidations_by_address.insert(hashed_address, invalid_nodes);
        }

        let account_logs: Vec<_> = update_batch
            .account_info_log_updates
            .iter()
            .cloned()
            .map(|(addr, old_info, new_info)| AccountInfoLogEntry {
                address: addr.0,
                info: new_info,
                previous_info: old_info,
            })
            .collect();

        let storage_logs: Vec<_> = update_batch.storage_log_updates.to_vec();

        {
            let mut store = self.inner()?;

            // Account info logs
            for log in account_logs {
                store
                    .account_state_logs
                    .entry(final_block)
                    .or_default()
                    .push((parent_block, log));
            }

            // Storage logs
            for storage_log in storage_logs {
                store
                    .account_storage_logs
                    .entry(final_block)
                    .or_default()
                    .push((parent_block, storage_log));
            }

            let current_snapshot = store.current_snapshot_block.unwrap_or_default();

            // Update flat tables if we're at the parent block
            if current_snapshot == parent_block {
                for (addr, _old_info, new_info) in
                    update_batch.account_info_log_updates.iter().cloned()
                {
                    if new_info == AccountInfo::default() {
                        store.account_info.remove(&addr.0);
                    } else {
                        store.account_info.insert(addr.0, new_info);
                    }
                }

                for entry in update_batch.storage_log_updates.iter().cloned() {
                    if entry.new_value.is_zero() {
                        store.account_storage.remove(&(entry.address, entry.slot));
                    } else {
                        store
                            .account_storage
                            .insert((entry.address, entry.slot), entry.new_value);
                    }
                }

                store.current_snapshot_block = Some(final_block);
            } else {
                tracing::warn!(
                    "Current snapshot block is not the parent block. Skipping update. Current snapshot: {:?}, Parent block: {:?}",
                    current_snapshot,
                    parent_block
                );
            }

            // Process state trie updates - separate ref counting from node insertion
            for (node_hash, _) in &state_trie_updates {
                // Increment reference counter
                *store.state_trie_ref_counters.entry(*node_hash).or_insert(0) += 1;
            }

            {
                let mut state_trie_store = store
                    .state_trie_nodes
                    .lock()
                    .map_err(|_| StoreError::LockError)?;

                for (node_hash, node_data) in state_trie_updates {
                    // Insert/update node data
                    state_trie_store.insert(node_hash, node_data);
                }
            }

            // State trie invalidations
            for node_hash in update_batch.invalidated_state_nodes {
                store
                    .state_trie_pruning_log
                    .entry(final_block)
                    .or_default()
                    .insert(node_hash.0);
            }

            // Code updates
            for (hashed_address, code) in update_batch.code_updates {
                store.account_codes.insert(hashed_address, code);
            }

            // Process storage trie ref counts first
            for (hashed_address, prepared_nodes) in &storage_updates_by_address {
                for (node_hash, _) in prepared_nodes {
                    // Increment reference counter for storage node
                    *store
                        .storage_trie_ref_counters
                        .entry((*hashed_address, *node_hash))
                        .or_insert(0) += 1;
                }
            }

            for (hashed_address, prepared_nodes) in storage_updates_by_address {
                // Add all the nodes for the specific address
                let mut addr_store = store
                    .storage_trie_nodes
                    .entry(hashed_address)
                    .or_default()
                    .lock()
                    .map_err(|_| StoreError::LockError)?;

                for (node_hash, node_data) in prepared_nodes {
                    tracing::debug!(
                        hashed_address = hex::encode(hashed_address.0),
                        stored_node_hash = hex::encode(node_hash),
                        "[STORING STORAGE NODE]"
                    );

                    addr_store.insert(node_hash, node_data);
                }
            }

            // Storage invalidations
            for (hashed_address, invalid_nodes) in storage_invalidations_by_address {
                let key_address: [u8; 32] = hashed_address.into();
                for node_hash in invalid_nodes {
                    store
                        .storage_trie_pruning_log
                        .entry(final_block)
                        .or_default()
                        .insert((key_address, NodeHash::Hashed(node_hash)));
                }
            }

            // Block storage
            for block in update_batch.blocks {
                let number = block.header.number;
                let hash = block.hash();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    store
                        .transaction_locations
                        .entry(transaction.compute_hash())
                        .or_default()
                        .push((number, hash, index as u64));
                }
                store.bodies.insert(hash, block.body);
                store.headers.insert(hash, block.header);
                store.block_numbers.insert(hash, number);
            }

            // Receipts
            for (block_hash, receipts) in update_batch.receipts {
                for (index, receipt) in receipts.into_iter().enumerate() {
                    store
                        .receipts
                        .entry(block_hash)
                        .or_default()
                        .insert(index as u64, receipt);
                }
            }
        }
        Ok(())

        // self.prune_state_and_storage_log()
    }

    async fn undo_writes_until_canonical(&self) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        let Some(mut current_snapshot) = store.current_snapshot_block else {
            return Ok(());
        };

        let mut block_num = current_snapshot.block_number;
        let mut snapshot_hash = current_snapshot.block_hash;

        let mut canonical_hash = store
            .canonical_hashes
            .get(&block_num)
            .copied()
            .unwrap_or_default();

        while canonical_hash != snapshot_hash {
            // Restore account info for the block of the current snapshot
            if let Some(entries) = store.account_state_logs.get(&current_snapshot).cloned() {
                for (parent_block, log) in entries {
                    // Restore previous state
                    if log.previous_info == AccountInfo::default() {
                        store.account_info.remove(&log.address);
                    } else {
                        store
                            .account_info
                            .insert(log.address, log.previous_info.clone());
                    }

                    // We update this here to ensure it's the previous block according
                    // to the logs found.
                    BlockNumHash {
                        block_number: block_num,
                        block_hash: snapshot_hash,
                    } = parent_block;
                }
            };

            // Restore account storage for the block of the current snapshot
            if let Some(entries) = store.account_storage_logs.get(&current_snapshot).cloned() {
                for (parent_block, log) in entries {
                    // Restore previous state
                    if log.old_value.is_zero() {
                        store.account_storage.remove(&(log.address, log.slot));
                    } else {
                        store
                            .account_storage
                            .insert((log.address, log.slot), log.old_value);
                    }

                    // We update this here to ensure it's the previous block according
                    // to the logs found.
                    BlockNumHash {
                        block_number: block_num,
                        block_hash: snapshot_hash,
                    } = parent_block;
                }
            };

            if current_snapshot == (block_num, snapshot_hash).into() {
                break;
            }

            // Get the canonical hash of the parent block
            canonical_hash = store
                .canonical_hashes
                .get(&block_num)
                .copied()
                .unwrap_or_default();

            // Update the current snapshot with the parent block
            current_snapshot = BlockNumHash {
                block_number: block_num,
                block_hash: snapshot_hash,
            };
        }

        store.current_snapshot_block = Some(current_snapshot);

        Ok(())
    }

    async fn replay_writes_until_head(&self, head_hash: H256) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        let Some(mut current_snapshot) = store.current_snapshot_block else {
            return Ok(());
        };

        // Asuming that we are in the bifurcation point, we start from the next block
        let start_block = current_snapshot.block_number + 1;

        for target_block_num in start_block.. {
            // Get the canonical hash for this block number
            let Some(canonical_hash) = store.canonical_hashes.get(&target_block_num).copied()
            else {
                break; // No more canonical blocks
            };

            let target_block = BlockNumHash {
                block_number: target_block_num,
                block_hash: canonical_hash,
            };

            tracing::warn!("REPLAY: processing block {target_block:?}");

            // Apply account state logs for this block
            if let Some(entries) = store.account_state_logs.get(&target_block).cloned() {
                for (parent_block, log) in entries {
                    // Verify this log applies to our current state
                    if parent_block != current_snapshot {
                        break;
                    }

                    // Apply the new state
                    if log.info == AccountInfo::default() {
                        store.account_info.remove(&log.address);
                    } else {
                        store.account_info.insert(log.address, log.info.clone());
                    }
                }
            }

            // Apply account storage logs for this block
            if let Some(entries) = store.account_storage_logs.get(&target_block).cloned() {
                for (parent_block, log) in entries {
                    // Verify this log applies to our current state
                    if parent_block != current_snapshot {
                        break;
                    }

                    // Apply the new state
                    if log.new_value.is_zero() {
                        store.account_storage.remove(&(log.address, log.slot));
                    } else {
                        store
                            .account_storage
                            .insert((log.address, log.slot), log.new_value);
                    }
                }
            }

            // Update current snapshot to the target block that we just processed
            current_snapshot = target_block;

            // Stop if we've reached the target head
            if canonical_hash == head_hash {
                tracing::warn!("REPLAY: reached head {head_hash:?}");
                break;
            }
        }

        // Update the current snapshot block
        store.current_snapshot_block = Some(current_snapshot);
        Ok(())
    }

    fn prune_state_and_storage_log(&self) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        // Get the block number of the last state trie pruning log entry
        if let Some(&max_block) = store.state_trie_pruning_log.keys().last() {
            let keep_from = max_block.block_number.saturating_sub(KEEP_BLOCKS);
            tracing::debug!(
                keep_from = keep_from,
                last_num = max_block.block_number,
                "[KEEPING STATE TRIE PRUNING LOG]"
            );

            // Get the blocks to remove from the state trie pruning log
            // from the start to the keep from block number
            let blocks_to_remove: Vec<_> = store
                .state_trie_pruning_log
                .iter()
                .take_while(|(block, _)| block.block_number < keep_from)
                .map(|(block, _)| *block)
                .collect();

            // Process each block and decrement counters directly
            for block in blocks_to_remove {
                let hashes = store
                    .state_trie_pruning_log
                    .remove(&block)
                    .ok_or(StoreError::LockError)?;

                let mut nodes_to_delete = Vec::new();

                // Get the nodes to delete from the state trie
                for hash in hashes {
                    let node_hash = NodeHash::Hashed(hash.into());

                    // Decrement reference counter for state node
                    if let Some(counter) = store.state_trie_ref_counters.get_mut(&node_hash) {
                        *counter -= 1;

                        // Only delete if reference count reaches 0
                        if *counter == 0 {
                            nodes_to_delete.push(node_hash);
                            store.state_trie_ref_counters.remove(&node_hash);
                        }
                    }
                }

                // Delete nodes from the state trie and the pruning log
                if !nodes_to_delete.is_empty() {
                    let mut trie = store
                        .state_trie_nodes
                        .lock()
                        .map_err(|_| StoreError::LockError)?;

                    for key in nodes_to_delete {
                        trie.remove(&key);
                        tracing::debug!(
                            node = hex::encode(key),
                            block_number = max_block.block_number,
                            block_hash = hex::encode(max_block.block_hash.0.as_ref()),
                            "[DELETING STATE NODE]"
                        );
                    }
                }
            }

            tracing::debug!(
                keep_from = keep_from,
                last_num = max_block.block_number,
                "[STOPPING STATE TRIE PRUNING]"
            );
        }

        // Get the block number of the last storage trie pruning log entry
        if let Some(&max_block) = store.storage_trie_pruning_log.keys().last() {
            let keep_from = max_block.block_number.saturating_sub(KEEP_BLOCKS);
            tracing::debug!(
                keep_from,
                last_num = max_block.block_number,
                "[KEEPING STORAGE TRIE PRUNING LOG]"
            );

            // Get the blocks to remove from the storage trie pruning log
            // from the start to the keep from block number
            let blocks_to_remove: Vec<_> = store
                .storage_trie_pruning_log
                .iter()
                .take_while(|(block, _)| block.block_number < keep_from)
                .map(|(block, _)| *block)
                .collect();

            // Process each block and decrement counters directly
            for block in blocks_to_remove {
                let entries = store
                    .storage_trie_pruning_log
                    .remove(&block)
                    .ok_or(StoreError::LockError)?;

                let mut storage_nodes_to_delete = Vec::new();

                // Get the nodes to delete from the storage trie
                for (addr_hash, node_hash) in entries {
                    let addr = H256(addr_hash);
                    let storage_key = (addr, node_hash);

                    // Decrement reference counter for storage node
                    if let Some(counter) = store.storage_trie_ref_counters.get_mut(&storage_key) {
                        *counter -= 1;

                        // Only delete if reference count reaches 0
                        if *counter == 0 {
                            storage_nodes_to_delete.push((addr, node_hash));
                            store.storage_trie_ref_counters.remove(&storage_key);
                        }
                    }
                }

                // Remove storage nodes from tries
                for (addr, key) in storage_nodes_to_delete {
                    if let Some(storage_store) = store.storage_trie_nodes.get(&addr) {
                        let mut trie = storage_store.lock().map_err(|_| StoreError::LockError)?;

                        trie.remove(&key);
                        tracing::debug!(
                            hashed_address = hex::encode(addr.0),
                            node_hash = hex::encode(key),
                            "[DELETING STORAGE NODE]"
                        );
                    }
                }
            }

            tracing::debug!(
                keep_from = keep_from,
                last_num = max_block.block_number,
                "[STOPPING STORAGE TRIE PRUNING]"
            );
        }

        Ok(())
    }

    fn get_current_account_info(
        &self,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let store = self.inner()?;
        Ok(store.account_info.get(&address).cloned())
    }

    async fn setup_genesis_flat_account_info(
        &self,
        genesis_block_number: u64,
        genesis_block_hash: H256,
        genesis_accounts: &[(Address, u64, U256, H256, bool)],
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        store.current_snapshot_block = Some(BlockNumHash {
            block_number: genesis_block_number,
            block_hash: genesis_block_hash,
        });

        for (address, nonce, balance, code_hash, removed) in genesis_accounts {
            if *removed {
                store.account_info.remove(address);
            } else {
                let account_info = AccountInfo {
                    nonce: *nonce,
                    balance: *balance,
                    code_hash: *code_hash,
                };
                store.account_info.insert(*address, account_info);
            }
        }

        Ok(())
    }

    fn get_block_for_current_snapshot(&self) -> Result<Option<BlockHash>, StoreError> {
        let store: MutexGuard<'_, StoreInner> = self.inner()?;
        Ok(store.current_snapshot_block.map(|block| block.block_hash))
    }

    fn get_current_storage(&self, address: Address, key: H256) -> Result<Option<U256>, StoreError> {
        let store = self.inner()?;
        Ok(store.account_storage.get(&(address, key)).cloned())
    }
    async fn setup_genesis_flat_account_storage(
        &self,
        genesis_block_number: u64,
        genesis_block_hash: H256,
        genesis_accounts: &[(Address, H256, U256)],
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        store.current_snapshot_block = Some(BlockNumHash {
            block_number: genesis_block_number,
            block_hash: genesis_block_hash,
        });

        for (address, slot, value) in genesis_accounts {
            if !value.is_zero() {
                store.account_storage.insert((*address, *slot), *value);
            } else {
                store.account_storage.remove(&(*address, *slot));
            }
        }

        Ok(())
    }
    fn get_block_header(&self, block_number: u64) -> Result<Option<BlockHeader>, StoreError> {
        let store = self.inner()?;
        if let Some(hash) = store.canonical_hashes.get(&block_number) {
            Ok(store.headers.get(hash).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_block_body(&self, block_number: u64) -> Result<Option<BlockBody>, StoreError> {
        let store = self.inner()?;
        if let Some(hash) = store.canonical_hashes.get(&block_number) {
            Ok(store.bodies.get(hash).cloned())
        } else {
            Ok(None)
        }
    }

    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let mut store = self.inner()?;
        let Some(hash) = store.canonical_hashes.get(&block_number).cloned() else {
            return Ok(());
        };
        store.canonical_hashes.remove(&block_number);
        store.block_numbers.remove(&hash);
        store.headers.remove(&hash);
        store.bodies.remove(&hash);
        Ok(())
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let store = self.inner()?;
        let mut res = Vec::new();
        for block_number in from..=to {
            if let Some(hash) = store.canonical_hashes.get(&block_number) {
                if let Some(block) = store.bodies.get(hash).cloned() {
                    res.push(block);
                }
            }
        }
        Ok(res)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let store = self.inner()?;
        let mut res = Vec::new();
        for hash in hashes {
            if let Some(block) = store.bodies.get(&hash).cloned() {
                res.push(block);
            }
        }
        Ok(res)
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.inner()?.pending_blocks.insert(block.hash(), block);
        Ok(())
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        Ok(self.inner()?.pending_blocks.get(&block_hash).cloned())
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.inner()?.headers.insert(block_hash, block_header);
        Ok(())
    }

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .headers
            .extend(block_hashes.into_iter().zip(block_headers));
        Ok(())
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.inner()?.bodies.insert(block_hash, block_body);
        Ok(())
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        for block in blocks {
            let header = block.header;
            let number = header.number;
            let hash = header.hash();
            let locations = block
                .body
                .transactions
                .iter()
                .enumerate()
                .map(|(i, tx)| (tx.compute_hash(), number, hash, i as u64));

            self.add_transaction_locations(locations.collect()).await?;
            self.add_block_body(hash, block.body.clone()).await?;
            self.add_block_header(hash, header).await?;
            self.add_block_number(hash, number).await?;
        }

        Ok(())
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        for block in blocks {
            self.inner()?
                .canonical_hashes
                .insert(block.header.number, block.hash());
        }

        Ok(())
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner()?.block_numbers.insert(block_hash, block_number);
        Ok(())
    }

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.block_numbers.get(&block_hash).copied())
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.get_block_number_sync(block_hash)
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.inner()?
            .transaction_locations
            .entry(transaction_hash)
            .or_default()
            .push((block_number, block_hash, index));
        Ok(())
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let store = self.inner()?;
        Ok(store
            .transaction_locations
            .get(&transaction_hash)
            .and_then(|v| {
                v.iter()
                    .find(|(number, hash, _index)| store.canonical_hashes.get(number) == Some(hash))
                    .copied()
            }))
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;
        let entry = store.receipts.entry(block_hash).or_default();
        entry.insert(index, receipt);
        Ok(())
    }

    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let store = self.inner()?;
        if let Some(hash) = store.canonical_hashes.get(&block_number) {
            Ok(store
                .receipts
                .get(hash)
                .and_then(|entry| entry.get(&index))
                .cloned())
        } else {
            Ok(None)
        }
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.inner()?.account_codes.insert(code_hash, code);
        Ok(())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        Ok(self.inner()?.account_codes.get(&code_hash).cloned())
    }

    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        // Store cancun timestamp
        self.inner()?.chain_data.chain_config = Some(*chain_config);
        Ok(())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        self.inner()?
            .chain_data
            .chain_config
            .ok_or(StoreError::Custom("No Chain Congif".to_string()))
    }

    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner()?
            .chain_data
            .earliest_block_number
            .replace(block_number);
        Ok(())
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.earliest_block_number)
    }

    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner()?
            .chain_data
            .finalized_block_number
            .replace(block_number);
        Ok(())
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.finalized_block_number)
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()?
            .chain_data
            .safe_block_number
            .replace(block_number);
        Ok(())
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.safe_block_number)
    }

    async fn update_latest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner()?
            .chain_data
            .latest_block_number
            .replace(block_number);
        Ok(())
    }
    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.latest_block_number)
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner()?
            .chain_data
            .pending_block_number
            .replace(block_number);
        Ok(())
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.pending_block_number)
    }

    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let mut store = self.inner()?;
        let trie_backend = store.storage_trie_nodes.entry(hashed_address).or_default();
        let db = Box::new(InMemoryTrieDB::new(trie_backend.clone()));
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let trie_backend = self.inner()?.state_trie_nodes.clone();
        let db = Box::new(InMemoryTrieDB::new(trie_backend));
        Ok(Trie::open(db, state_root))
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        Ok(self.inner()?.bodies.get(&block_hash).cloned())
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        Ok(self.inner()?.headers.get(&block_hash).cloned())
    }

    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.inner()?.canonical_hashes.insert(number, hash);
        Ok(())
    }

    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.inner()?.canonical_hashes.get(&block_number).cloned())
    }

    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.get_canonical_block_hash_sync(block_number)
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.inner()?.canonical_hashes.remove(&number);
        Ok(())
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.inner()?
            .payloads
            .insert(payload_id, PayloadBundle::from_block(block));
        Ok(())
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        Ok(self.inner()?.payloads.get(&payload_id).cloned())
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let store = self.inner()?;
        let Some(receipts_for_block) = store.receipts.get(block_hash) else {
            return Ok(vec![]);
        };
        let mut receipts = receipts_for_block
            .iter()
            .collect::<Vec<(&Index, &Receipt)>>();

        receipts.sort_by_key(|(index, _receipt)| **index);

        Ok(receipts
            .into_iter()
            .map(|(_index, receipt)| receipt.clone())
            .collect())
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;
        let entry = store.receipts.entry(block_hash).or_default();
        for (index, receipt) in receipts.into_iter().enumerate() {
            entry.insert(index as u64, receipt);
        }
        Ok(())
    }

    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        for (transaction_hash, block_number, block_hash, index) in locations {
            self.inner()?
                .transaction_locations
                .entry(transaction_hash)
                .or_default()
                .push((block_number, block_hash, index));
        }

        Ok(())
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.inner()?.payloads.insert(payload_id, payload);
        Ok(())
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.inner()?.snap_state.header_download_checkpoint = Some(block_hash);
        Ok(())
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.inner()?.snap_state.header_download_checkpoint)
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.inner()?.snap_state.state_trie_key_checkpoint = Some(last_keys);
        Ok(())
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        Ok(self.inner()?.snap_state.state_trie_key_checkpoint)
    }

    async fn set_storage_heal_paths(
        &self,
        paths: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .snap_state
            .storage_heal_paths
            .get_or_insert(Default::default())
            .extend(paths);
        Ok(())
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        Ok(self
            .inner()?
            .snap_state
            .storage_heal_paths
            .as_mut()
            .map(|paths| paths.drain(..limit).collect())
            .unwrap_or_default())
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        self.inner()?.snap_state = Default::default();
        Ok(())
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.inner()?.snap_state.state_heal_paths = Some(paths);
        Ok(())
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        Ok(self.inner()?.snap_state.state_heal_paths.clone())
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<ethrex_common::types::AccountState>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .state_snapshot
            .extend(account_hashes.into_iter().zip(account_states));
        Ok(())
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        self.inner()?
            .storage_snapshot
            .entry(account_hash)
            .or_default()
            .extend(storage_keys.into_iter().zip(storage_values));
        Ok(())
    }
    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        for (account_hash, (storage_keys, storage_values)) in account_hashes
            .into_iter()
            .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
        {
            self.inner()?
                .storage_snapshot
                .entry(account_hash)
                .or_default()
                .extend(storage_keys.into_iter().zip(storage_values));
        }
        Ok(())
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.inner()?.snap_state.state_trie_rebuild_checkpoint = Some(checkpoint);
        Ok(())
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        Ok(self.inner()?.snap_state.state_trie_rebuild_checkpoint)
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        self.inner()?.snap_state.state_trie_rebuild_checkpoint = None;
        self.inner()?.snap_state.storage_trie_rebuild_pending = None;
        Ok(())
    }

    fn read_account_snapshot(
        &self,
        start: H256,
    ) -> Result<Vec<(H256, ethrex_common::types::AccountState)>, StoreError> {
        Ok(self
            .inner()?
            .state_snapshot
            .iter()
            .filter(|(hash, _)| **hash < start)
            .take(MAX_SNAPSHOT_READS)
            .map(|(h, a)| (*h, a.clone()))
            .collect())
    }

    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        if let Some(snapshot) = self.inner()?.storage_snapshot.get(&account_hash) {
            Ok(snapshot
                .iter()
                .filter(|(hash, _)| **hash < start)
                .take(MAX_SNAPSHOT_READS)
                .map(|(k, v)| (*k, *v))
                .collect())
        } else {
            Ok(vec![])
        }
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.inner()?.snap_state.storage_trie_rebuild_pending = Some(pending);
        Ok(())
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        Ok(self
            .inner()?
            .snap_state
            .storage_trie_rebuild_pending
            .clone())
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.inner()?.invalid_ancestors.get(&block).cloned())
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        self.inner()?
            .invalid_ancestors
            .insert(bad_block, latest_valid);
        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory Store").finish()
    }
}
