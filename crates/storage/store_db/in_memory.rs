use crate::{
    UpdateBatch,
    api::StoreEngine,
    error::StoreError,
    store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS, TrieUpdates},
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
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard, RwLock},
};
use tracing::{debug, info, warn};
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
    account_state_logs: HashMap<BlockNumHash, Vec<(BlockNumHash, AccountInfoLogEntry)>>,
    account_storage_logs: HashMap<BlockNumHash, Vec<(BlockNumHash, AccountStorageLogEntry)>>,
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
        Self::default()
    }
    fn inner(&self) -> Result<MutexGuard<'_, StoreInner>, StoreError> {
        self.0.lock().map_err(|_| StoreError::LockError)
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_trie_updates(&self, _trie_updates: TrieUpdates) -> Result<(), StoreError> {
        todo!()
    }

    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        let (Some(first_block), Some(last_block)) =
            (update_batch.blocks.first(), update_batch.blocks.last())
        else {
            return Ok(());
        };
        let parent_block = (
            first_block.header.number - 1,
            first_block.header.parent_hash,
        )
            .into();

        let final_block = (last_block.header.number, last_block.hash()).into();
        for (addr, old_info, new_info) in update_batch.account_info_log_updates.iter().cloned() {
            let log = AccountInfoLogEntry {
                address: addr.0,
                info: new_info,
                previous_info: old_info,
            };
            store
                .account_state_logs
                .insert(final_block, vec![(parent_block, log)]);
        }

        for storage_log in update_batch.storage_log_updates.iter().cloned() {
            store
                .account_storage_logs
                .insert(final_block, vec![(parent_block, storage_log)]);
        }

        let current_spanshot = store.current_snapshot_block.unwrap_or_default();

        // If the current snapshot is the parent block, we can update the account and storage
        if current_spanshot == parent_block {
            for (addr, _old_info, new_info) in update_batch.account_info_log_updates.iter().cloned()
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
        }

        // store code updates
        for (hashed_address, code) in update_batch.code_updates {
            store.account_codes.insert(hashed_address, code);
        }

        for block in update_batch.blocks {
            // store block
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

        for (block_hash, receipts) in update_batch.receipts {
            for (index, receipt) in receipts.into_iter().enumerate() {
                store
                    .receipts
                    .entry(block_hash)
                    .or_default()
                    .insert(index as u64, receipt);
            }
        }

        Ok(())
    }

    /// Rewinds (a.k.a undo) writes from the write logs until a canonical block is reached.
    ///
    /// This is used to restore the flat tables from the write logs after a reorg.
    async fn undo_writes_until_canonical(&self) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        // Get the current snapshot block
        let Some(mut current_snapshot) = store.current_snapshot_block else {
            info!("No current snapshot block found, nothing to undo");
            return Ok(());
        };

        let mut block_num = current_snapshot.0;
        let mut snapshot_hash = current_snapshot.1;

        let mut canonical_hash = store
            .canonical_hashes
            .get(&block_num)
            .copied()
            .unwrap_or_default();

        while canonical_hash != snapshot_hash {
            warn!("UNDO: searching for {current_snapshot:?}");

            // Restore account info for the block of the current snapshot
            if let Some(entries) = store.account_state_logs.get(&current_snapshot).cloned() {
                for (parent_block, log) in entries {
                    // Avoid infinite loop
                    if current_snapshot == parent_block {
                        break;
                    }
                    // Restore previous state
                    if log.previous_info == AccountInfo::default() {
                        debug!("UNDO: removing account info for {:?}", log.address);
                        store.account_info.remove(&log.address);
                    } else {
                        debug!("UNDO: restoring account info for {:?}", log.address);
                        store
                            .account_info
                            .insert(log.address, log.previous_info.clone());
                    }

                    BlockNumHash(block_num, snapshot_hash) = parent_block;
                }
            };

            // Restore account storage for the block of the current snapshot
            if let Some(entries) = store.account_storage_logs.get(&current_snapshot).cloned() {
                for (parent_block, log) in entries {
                    // Avoid infinite loop
                    if current_snapshot == parent_block {
                        break;
                    }
                    // Restore previous state
                    if log.old_value.is_zero() {
                        debug!("UNDO: removing account storage for {:?}", log.address);
                        store.account_storage.remove(&(log.address, log.slot));
                    } else {
                        debug!("UNDO: restoring account storage for {:?}", log.address);
                        store
                            .account_storage
                            .insert((log.address, log.slot), log.old_value);
                    }
                    // Move to the parent block
                    BlockNumHash(block_num, snapshot_hash) = parent_block;
                }
            };

            canonical_hash = store
                .canonical_hashes
                .get(&block_num)
                .copied()
                .unwrap_or_default();

            current_snapshot = BlockNumHash(block_num, snapshot_hash);
        }

        store.current_snapshot_block = Some(current_snapshot);
        info!("UNDO: current snapshot block set to {:?}", current_snapshot);

        Ok(())
    }

    /// Replays writes from the write logs until the head block is reached.
    ///
    /// This is used to restore the flat tables from the write logs after a reorg.
    /// Assumes that the current flat representation corresponds to a block in the canonical chain.
    /// *NOTE:* this function is meant to be called after calling `undo_writes_until_canonical` to
    /// restore the flat tables to stay in sync with the canonical chain after a reorg.
    ///
    /// # Arguments
    ///
    ///  * `head_hash` - The block hash of the head block to replay writes until.
    async fn replay_writes_until_head(&self, head_hash: H256) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        let Some(mut current_snapshot) = store.current_snapshot_block else {
            warn!("REPLAY: No current snapshot block found");
            return Ok(());
        };

        // Iterate through canonical blocks starting from the next block after current snapshot
        let start_block = current_snapshot.0 + 1;

        for target_block_num in start_block.. {
            // Get the canonical hash for this block number
            let Some(canonical_hash) = store.canonical_hashes.get(&target_block_num).copied()
            else {
                break; // No more canonical blocks
            };

            let target_block = BlockNumHash(target_block_num, canonical_hash);

            let has_state = store
                .account_state_logs
                .get(&target_block)
                .map(|entries| {
                    entries
                        .iter()
                        .any(|(parent, _)| *parent == current_snapshot)
                })
                .unwrap_or(false);

            let has_storage = store
                .account_storage_logs
                .get(&target_block)
                .map(|entries| {
                    entries
                        .iter()
                        .any(|(parent, _)| *parent == current_snapshot)
                })
                .unwrap_or(false);

            // If there are no logs for this block, skip it
            if !has_state && !has_storage {
                info!("REPLAY: skipping block since it has no logs {target_block:?}");
                continue;
            }

            warn!("REPLAY: processing block {target_block:?}");

            // Apply account state logs for this block
            if let Some(entries) = store.account_state_logs.get(&target_block).cloned() {
                for (parent_block, log) in entries {
                    // Verify this log applies to our current state
                    if parent_block != current_snapshot {
                        continue;
                    }

                    // Apply the new state
                    if log.info == AccountInfo::default() {
                        debug!("REPLAY: removing account info for {:?}", log.address);
                        store.account_info.remove(&log.address);
                    } else {
                        debug!("REPLAY: applying account info for {:?}", log.address);
                        store.account_info.insert(log.address, log.info.clone());
                    }
                }
            }

            // Apply account storage logs for this block
            if let Some(entries) = store.account_storage_logs.get(&target_block).cloned() {
                for (parent_block, log) in entries {
                    // Verify this log applies to our current state
                    if parent_block != current_snapshot {
                        continue;
                    }

                    // Apply the new state
                    if log.new_value.is_zero() {
                        debug!("REPLAY: removing account storage for {:?}", log.address);
                        store.account_storage.remove(&(log.address, log.slot));
                    } else {
                        debug!("REPLAY: applying account storage for {:?}", log.address);
                        store
                            .account_storage
                            .insert((log.address, log.slot), log.new_value);
                    }
                }
            }

            // Update current snapshot to this block
            current_snapshot = target_block;

            // Stop if we've reached the target head
            if canonical_hash == head_hash {
                info!("REPLAY: Reached target head {head_hash:?}");
                break;
            }
        }

        // Update the current snapshot block
        store.current_snapshot_block = Some(current_snapshot);
        info!(
            "REPLAY: current snapshot block set to {:?}",
            current_snapshot
        );

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

        store.current_snapshot_block = Some(BlockNumHash(genesis_block_number, genesis_block_hash));

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
        Ok(store.current_snapshot_block.map(|block| block.1))
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

        store.current_snapshot_block = Some(BlockNumHash(genesis_block_number, genesis_block_hash));

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
        _dirty_storage_nodes: Arc<RwLock<HashMap<([u8; 32], NodeHash), Vec<u8>>>>,
    ) -> Result<Trie, StoreError> {
        let mut store = self.inner()?;
        let trie_backend = store.storage_trie_nodes.entry(hashed_address).or_default();
        let db = Box::new(InMemoryTrieDB::new(trie_backend.clone()));
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(
        &self,
        state_root: H256,
        _dirty_state_nodes: Arc<RwLock<HashMap<NodeHash, Vec<u8>>>>,
    ) -> Result<Trie, StoreError> {
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
