use crate::{
    UpdateBatch,
    api::StoreEngine,
    apply_prefix,
    error::{SnapshotGenerationError, StoreError},
    store::STATE_TRIE_SEGMENTS,
    trie_db::layering::{TrieLayerCache, TrieWrapper},
};
use bytes::Bytes;
use ethereum_types::H256;
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
    Receipt,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{InMemoryTrieDB, Nibbles, Node, Trie, db::NodeMap};
use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard, RwLock},
};

#[derive(Default, Clone)]
pub struct Store(Arc<Mutex<StoreInner>>);

#[derive(Default, Debug)]
pub struct StoreInner {
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
    trie_cache: Arc<RwLock<TrieLayerCache>>,
    // Contains account trie nodes
    state_trie_nodes: NodeMap,
    pending_blocks: HashMap<BlockHash, Block>,
    // Stores invalid blocks and their latest valid ancestor
    invalid_ancestors: HashMap<BlockHash, BlockHash>,
    // Stores current Snap State
    snap_state: SnapState,
    // Is snapshot complete
    snapshot_completed: bool,
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
    /// State trie Paths in need of healing
    state_heal_paths: Option<Vec<(Nibbles, H256)>>,
    /// Storage tries waiting rebuild
    storage_trie_rebuild_pending: Option<Vec<(H256, H256)>>,
    // Latest root of the rebuilt state trie + the last inserted keys for each state trie segment
    state_trie_rebuild_checkpoint: Option<(H256, [H256; STATE_TRIE_SEGMENTS])>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn inner(&self) -> Result<MutexGuard<'_, StoreInner>, StoreError> {
        self.0.lock().map_err(|_| StoreError::LockError)
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        // Store trie updates
        {
            let mut trie = store
                .trie_cache
                .write()
                .map_err(|_| StoreError::LockError)?;
            let parent_state_root = self
                .get_block_header_by_hash(
                    update_batch
                        .blocks
                        .first()
                        .ok_or(StoreError::UpdateBatchNoBlocks)?
                        .header
                        .parent_hash,
                )?
                .map(|header| header.state_root)
                .unwrap_or_default();

            let last_state_root = update_batch
                .blocks
                .last()
                .ok_or(StoreError::UpdateBatchNoBlocks)?
                .header
                .state_root;

            let mut state_trie = store
                .state_trie_nodes
                .lock()
                .map_err(|_| StoreError::LockError)?;

            if let Some(root) = trie.get_commitable(parent_state_root) {
                let nodes = trie.commit(root).unwrap_or_default();
                for (key, value) in nodes {
                    if value.is_empty() {
                        state_trie.remove(&key);
                    } else {
                        state_trie.insert(key, value);
                    }
                }
            }
            trie.put_batch(
                parent_state_root,
                last_state_root,
                update_batch
                    .storage_updates
                    .into_iter()
                    .flat_map(|(account_hash, nodes)| {
                        nodes
                            .into_iter()
                            .map(move |(path, node)| (apply_prefix(Some(account_hash), path), node))
                    })
                    .chain(update_batch.account_updates)
                    .collect(),
            );
        }

        for block in update_batch.blocks {
            // store block
            let number = block.header.number;
            let hash = block.hash();

            for (index, transaction) in block.body.transactions.iter().enumerate() {
                store
                    .transaction_locations
                    .entry(transaction.hash())
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
            if let Some(block) = store
                .canonical_hashes
                .get(&block_number)
                .and_then(|hash| store.bodies.get(hash))
            {
                res.push(block.clone())
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

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        self.inner()?.snap_state = Default::default();
        Ok(())
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
        let block_number = block_header.number;
        self.add_block_number(block_hash, block_number).await?;
        self.inner()?.headers.insert(block_hash, block_header);
        Ok(())
    }

    async fn add_block_headers(&self, block_headers: Vec<BlockHeader>) -> Result<(), StoreError> {
        self.inner()?.block_numbers.extend(
            block_headers
                .iter()
                .map(|header| (header.hash(), header.number)),
        );
        self.inner()?.headers.extend(
            block_headers
                .into_iter()
                .map(|header| (header.hash(), header)),
        );
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
                .map(|(i, tx)| (number, tx.hash(), i as u64))
                .collect();
            self.inner()?.transaction_locations.insert(hash, locations);
            self.add_block_body(hash, block.body.clone()).await?;
            self.add_block_header(hash, header).await?;
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
        block_hash: BlockHash,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let store = self.inner()?;
        Ok(store
            .receipts
            .get(&block_hash)
            .and_then(|entry| entry.get(&index))
            .cloned())
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

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.latest_block_number)
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.earliest_block_number)
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.finalized_block_number)
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner()?.chain_data.safe_block_number)
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
        _storage_root: H256,
        state_root: H256,
    ) -> Result<Trie, StoreError> {
        let store = self.inner()?;
        let trie_backend = store.state_trie_nodes.clone();
        let db = Box::new(InMemoryTrieDB::new(trie_backend));
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: store.trie_cache.clone(),
            db,
            prefix: Some(hashed_address),
            snapshot_completed: store.snapshot_completed,
        });
        Ok(Trie::open(wrap_db, state_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let store = self.inner()?;
        let trie_backend = store.state_trie_nodes.clone();
        let db = Box::new(InMemoryTrieDB::new(trie_backend));
        let wrap_db = Box::new(TrieWrapper {
            state_root,
            inner: store.trie_cache.clone(),
            db,
            prefix: None,
            snapshot_completed: store.snapshot_completed,
        });
        Ok(Trie::open(wrap_db, state_root))
    }

    fn open_direct_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let store = self.inner()?;
        let trie_backend = store.state_trie_nodes.clone();
        let db = Box::new(InMemoryTrieDB::new(trie_backend));
        Ok(Trie::open(db, state_root))
    }

    fn open_direct_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let store = self.inner()?;
        let trie_backend = store.state_trie_nodes.clone();
        let prefix = apply_prefix(Some(hashed_address), Default::default());
        let db = Box::new(InMemoryTrieDB::new_with_prefix(trie_backend, prefix));
        Ok(Trie::open(db, storage_root))
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

    async fn forkchoice_update(
        &self,
        new_canonical_blocks: Option<Vec<(BlockNumber, BlockHash)>>,
        head_number: BlockNumber,
        head_hash: BlockHash,
        safe: Option<BlockNumber>,
        finalized: Option<BlockNumber>,
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        // Make all ancestors to head canonical.
        if let Some(new_canonical_blocks) = new_canonical_blocks {
            for (number, hash) in new_canonical_blocks {
                store.canonical_hashes.insert(number, hash);
            }
        }

        // Remove anything after the head from the canonical chain.
        let latest = store.chain_data.latest_block_number.unwrap_or(0);
        for number in (head_number + 1)..(latest + 1) {
            store.canonical_hashes.remove(&number);
        }

        // Make head canonical and label all special blocks correctly.
        store.canonical_hashes.insert(head_number, head_hash);

        if let Some(finalized) = finalized {
            store.chain_data.finalized_block_number.replace(finalized);
        }

        if let Some(safe) = safe {
            store.chain_data.safe_block_number.replace(safe);
        }

        store.chain_data.latest_block_number.replace(head_number);

        Ok(())
    }

    async fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Vec<Receipt>, StoreError> {
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

    async fn set_state_heal_paths(&self, paths: Vec<(Nibbles, H256)>) -> Result<(), StoreError> {
        self.inner()?.snap_state.state_heal_paths = Some(paths);
        Ok(())
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<(Nibbles, H256)>>, StoreError> {
        Ok(self.inner()?.snap_state.state_heal_paths.clone())
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

    async fn write_storage_trie_nodes_batch(
        &self,
        storage_trie_nodes: Vec<(H256, Vec<(Nibbles, Vec<u8>)>)>,
    ) -> Result<(), StoreError> {
        let store = self.inner()?;
        let mut trie = store
            .state_trie_nodes
            .lock()
            .map_err(|_| StoreError::LockError)?;

        for (hashed_address, nodes) in storage_trie_nodes {
            for (node_path, node_data) in nodes {
                let full_path = apply_prefix(Some(hashed_address), node_path);
                trie.insert(full_path.into_vec(), node_data);
            }
        }

        Ok(())
    }

    async fn write_account_code_batch(
        &self,
        account_codes: Vec<(H256, Bytes)>,
    ) -> Result<(), StoreError> {
        let mut store = self.inner()?;

        for (code_hash, code) in account_codes {
            store.account_codes.insert(code_hash, code);
        }

        Ok(())
    }

    async fn generate_snapshot(&self, state_root: H256) -> Result<(), StoreError> {
        if self.inner()?.snapshot_completed {
            return Ok(());
        }
        // Open the account state trie
        let account_state_trie = self
            .open_direct_state_trie(state_root)
            .map_err(Box::new)
            .map_err(SnapshotGenerationError::FailedToOpenAccountStateTrie)?;

        // Retrieve account state db
        let account_state_db = account_state_trie.db();

        let mut nodes_to_write = Vec::new();

        // Traverse account state trie
        self
            .open_direct_state_trie(state_root)
            .map_err(Box::new)
            .map_err(SnapshotGenerationError::FailedToOpenAccountStateTrie)?
            .into_iter()
            .try_for_each(|(path, node)| -> Result<(), StoreError> {
                // Retrieve account state node
                let Node::Leaf(node) = node else {
                    return Ok(());
                };

                let path_as_key = H256::from_slice(&path.to_bytes());

                // Retrieve the account state by decoding the node
                let AccountState {
                    storage_root: account_hash,
                    ..
                } = AccountState::decode(&node.value).map_err(|rlp_decode_err| {
                    SnapshotGenerationError::FailedToDecodeAccountState(path_as_key, rlp_decode_err)
                })?;

                // Open the account state storage trie
                let storage_trie = self
                    .open_direct_storage_trie(path_as_key, account_hash)
                    .map_err(Box::new)
                    .map_err(|store_err| {
                        SnapshotGenerationError::FailedToOpenAccountStateStorageTrie(
                            account_hash,
                            path_as_key,
                            store_err,
                        )
                    })?;

                // Retrieve the account state storage db
                let storage_trie_db = storage_trie.db();

                let mut storages_to_write = Vec::new();

                // Traverse account state storage trie
                self
                    .open_direct_storage_trie(path_as_key, account_hash)
                    .map_err(Box::new)
                    .map_err(|store_err| {
                        SnapshotGenerationError::FailedToOpenAccountStateStorageTrie(
                            account_hash,
                            path_as_key,
                            store_err,
                        )
                    })?
                    .into_iter()
                    .try_for_each(|(path, node)| -> Result<(), StoreError> {
                        // Retrieve the account state storage node
                        let Node::Leaf(node) = node else {
                            return Ok(());
                        };

                        // Add to the list of storage nodes to store
                        storages_to_write.push((path, node.value));

                        // Store every 100k batches account storage node batches
                        if storages_to_write.len() > 100_000 {
                            storage_trie_db
                                .put_batch(std::mem::take(&mut storages_to_write))
                                .map_err(|trie_err| {
                                    SnapshotGenerationError::FailedToStoreAccountStateStorageNodesBatch(
                                        account_hash,
                                        trie_err,
                                    )
                                })?;
                        }

                        Ok(())
                    })?;

                // Store the remaining account storage nodes
                storage_trie_db
                    .put_batch(storages_to_write)
                    .map_err(|trie_err| {
                        SnapshotGenerationError::FailedToStoreRemainingAccountStateStorageNodes(
                            account_hash,
                            trie_err,
                        )
                    })?;

                // Add account state node to the list of account state nodes to store
                nodes_to_write.push((path, node.value));

                // Store every in 100k account state nodes batches
                if nodes_to_write.len() > 100_000 {
                    account_state_db
                        .put_batch(std::mem::take(&mut nodes_to_write))
                        .map_err(|trie_err| {
                            SnapshotGenerationError::FailedToStoreAccountStateNodesBatch(
                                account_hash,
                                trie_err,
                            )
                        })?;
                }

                Ok(())
            })?;

        // Store the remaining account state nodes
        account_state_db
            .put_batch(nodes_to_write)
            .map_err(SnapshotGenerationError::FailedToStoreRemainingAccountStateNodes)?;

        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory Store").finish()
    }
}
