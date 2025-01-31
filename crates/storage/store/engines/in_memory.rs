use crate::error::StoreError;
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_core::types::{
    BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
};
use ethrex_trie::{InMemoryTrieDB, Nibbles, Trie};
use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
};

use super::api::StoreEngine;

pub type NodeMap = Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>;

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
    // TODO (#307): Remove TotalDifficulty.
    block_total_difficulties: HashMap<BlockHash, U256>,
    // Stores local blocks by payload id
    payloads: HashMap<u64, (Block, U256, BlobsBundle, bool)>,
    pending_blocks: HashMap<BlockHash, Block>,
    // Stores current Snap Sate
    snap_state: SnapState,
}

#[derive(Default, Debug)]
struct ChainData {
    chain_config: Option<ChainConfig>,
    earliest_block_number: Option<BlockNumber>,
    finalized_block_number: Option<BlockNumber>,
    safe_block_number: Option<BlockNumber>,
    latest_block_number: Option<BlockNumber>,
    // TODO (#307): Remove TotalDifficulty.
    latest_total_difficulty: Option<U256>,
    pending_block_number: Option<BlockNumber>,
    is_synced: bool,
}

// Keeps track of the state left by the latest snap attempt
#[derive(Default, Debug)]
pub struct SnapState {
    /// Latest downloaded block header's hash from a previously aborted sync
    header_download_checkpoint: Option<BlockHash>,
    /// Current root hash of the latest State Trie (Used for both fetching and healing)
    state_trie_root_checkpoint: Option<H256>,
    /// Last downloaded key of the latest State Trie
    state_trie_key_checkpoint: Option<H256>,
    /// Accounts which storage needs healing
    pending_storage_heal_accounts: Option<Vec<(H256, Vec<Nibbles>)>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }
    fn inner(&self) -> MutexGuard<'_, StoreInner> {
        self.0.lock().unwrap()
    }
}

impl StoreEngine for Store {
    fn get_block_header(&self, block_number: u64) -> Result<Option<BlockHeader>, StoreError> {
        let store = self.inner();
        if let Some(hash) = store.canonical_hashes.get(&block_number) {
            Ok(store.headers.get(hash).cloned())
        } else {
            Ok(None)
        }
    }

    fn get_block_body(&self, block_number: u64) -> Result<Option<BlockBody>, StoreError> {
        let store = self.inner();
        if let Some(hash) = store.canonical_hashes.get(&block_number) {
            Ok(store.bodies.get(hash).cloned())
        } else {
            Ok(None)
        }
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.inner()
            .pending_blocks
            .insert(block.header.compute_block_hash(), block);
        Ok(())
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        Ok(self.inner().pending_blocks.get(&block_hash).cloned())
    }

    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.inner().headers.insert(block_hash, block_header);
        Ok(())
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        self.inner()
            .headers
            .extend(block_hashes.into_iter().zip(block_headers));
        Ok(())
    }

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.inner().bodies.insert(block_hash, block_body);
        Ok(())
    }

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.inner().block_numbers.insert(block_hash, block_number);
        Ok(())
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().block_numbers.get(&block_hash).copied())
    }

    fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        self.inner()
            .block_total_difficulties
            .insert(block_hash, block_total_difficulty);
        Ok(())
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<U256>, StoreError> {
        Ok(self
            .inner()
            .block_total_difficulties
            .get(&block_hash)
            .copied())
    }

    fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.inner()
            .transaction_locations
            .entry(transaction_hash)
            .or_default()
            .push((block_number, block_hash, index));
        Ok(())
    }

    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let store = self.inner();
        Ok(store
            .transaction_locations
            .get(&transaction_hash)
            .and_then(|v| {
                v.iter()
                    .find(|(number, hash, _index)| store.canonical_hashes.get(number) == Some(hash))
                    .copied()
            }))
    }

    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let mut store = self.inner();
        let entry = store.receipts.entry(block_hash).or_default();
        entry.insert(index, receipt);
        Ok(())
    }

    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let store = self.inner();
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

    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.inner().account_codes.insert(code_hash, code);
        Ok(())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        Ok(self.inner().account_codes.get(&code_hash).cloned())
    }

    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        // Store cancun timestamp
        self.inner().chain_data.chain_config = Some(*chain_config);
        Ok(())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        Ok(self.inner().chain_data.chain_config.unwrap())
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .earliest_block_number
            .replace(block_number);
        Ok(())
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().chain_data.earliest_block_number)
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .finalized_block_number
            .replace(block_number);
        Ok(())
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().chain_data.finalized_block_number)
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .safe_block_number
            .replace(block_number);
        Ok(())
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().chain_data.safe_block_number)
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .latest_block_number
            .replace(block_number);
        Ok(())
    }
    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .latest_total_difficulty
            .replace(latest_total_difficulty);
        Ok(())
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().chain_data.latest_block_number)
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.inner()
            .chain_data
            .pending_block_number
            .replace(block_number);
        Ok(())
    }

    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError> {
        Ok(self.inner().chain_data.latest_total_difficulty)
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self.inner().chain_data.pending_block_number)
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        let mut store = self.inner();
        let trie_backend = store.storage_trie_nodes.entry(hashed_address).or_default();
        let db = Box::new(InMemoryTrieDB::new(trie_backend.clone()));
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        let trie_backend = self.inner().state_trie_nodes.clone();
        let db = Box::new(InMemoryTrieDB::new(trie_backend));
        Trie::open(db, state_root)
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        Ok(self.inner().bodies.get(&block_hash).cloned())
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        Ok(self.inner().headers.get(&block_hash).cloned())
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        self.inner().canonical_hashes.insert(number, hash);
        Ok(())
    }

    fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.inner().canonical_hashes.get(&block_number).cloned())
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.inner().canonical_hashes.remove(&number);
        Ok(())
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.inner().payloads.insert(
            payload_id,
            (block, U256::zero(), BlobsBundle::empty(), false),
        );
        Ok(())
    }

    fn get_payload(
        &self,
        payload_id: u64,
    ) -> Result<Option<(Block, U256, BlobsBundle, bool)>, StoreError> {
        Ok(self.inner().payloads.get(&payload_id).cloned())
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let store = self.inner();
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

    fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut store = self.inner();
        let entry = store.receipts.entry(block_hash).or_default();
        for (index, receipt) in receipts.into_iter().enumerate() {
            entry.insert(index as u64, receipt);
        }
        Ok(())
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        for (transaction_hash, block_number, block_hash, index) in locations {
            self.inner()
                .transaction_locations
                .entry(transaction_hash)
                .or_default()
                .push((block_number, block_hash, index));
        }

        Ok(())
    }
    fn update_payload(
        &self,
        payload_id: u64,
        block: Block,
        block_value: U256,
        blobs_bundle: BlobsBundle,
        completed: bool,
    ) -> Result<(), StoreError> {
        self.inner()
            .payloads
            .insert(payload_id, (block, block_value, blobs_bundle, completed));
        Ok(())
    }

    fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        self.inner().snap_state.header_download_checkpoint = Some(block_hash);
        Ok(())
    }

    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.inner().snap_state.header_download_checkpoint)
    }

    fn clear_header_download_checkpoint(&self) -> Result<(), StoreError> {
        self.inner().snap_state.header_download_checkpoint = None;
        Ok(())
    }

    fn set_state_trie_root_checkpoint(&self, current_root: H256) -> Result<(), StoreError> {
        self.inner().snap_state.state_trie_root_checkpoint = Some(current_root);
        Ok(())
    }

    fn get_state_trie_root_checkpoint(&self) -> Result<Option<H256>, StoreError> {
        Ok(self.inner().snap_state.state_trie_root_checkpoint)
    }

    fn clear_state_trie_root_checkpoint(&self) -> Result<(), StoreError> {
        self.inner().snap_state.state_trie_root_checkpoint = None;
        Ok(())
    }

    fn set_state_trie_key_checkpoint(&self, last_key: H256) -> Result<(), StoreError> {
        self.inner().snap_state.state_trie_key_checkpoint = Some(last_key);
        Ok(())
    }

    fn get_state_trie_key_checkpoint(&self) -> Result<Option<H256>, StoreError> {
        Ok(self.inner().snap_state.state_trie_key_checkpoint)
    }

    fn clear_state_trie_key_checkpoint(&self) -> Result<(), StoreError> {
        self.inner().snap_state.state_trie_key_checkpoint = None;
        Ok(())
    }

    fn set_pending_storage_heal_accounts(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        self.inner().snap_state.pending_storage_heal_accounts = Some(accounts);
        Ok(())
    }

    fn get_pending_storage_heal_accounts(
        &self,
    ) -> Result<Option<Vec<(H256, Vec<Nibbles>)>>, StoreError> {
        Ok(self
            .inner()
            .snap_state
            .pending_storage_heal_accounts
            .clone())
    }

    fn clear_pending_storage_heal_accounts(&self) -> Result<(), StoreError> {
        self.inner().snap_state.pending_storage_heal_accounts = None;
        Ok(())
    }

    fn is_synced(&self) -> Result<bool, StoreError> {
        Ok(self.inner().chain_data.is_synced)
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        self.inner().chain_data.is_synced = status;
        Ok(())
    }
    
    fn set_state_heal_paths(&self, paths: Vec<ethrex_trie::Nibbles>) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_heal_paths(&self) -> Result<Option<Vec<ethrex_trie::Nibbles>>, StoreError> {
        todo!()
    }

    fn clear_state_heal_paths(&self) -> Result<(), StoreError> {
        todo!()
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory Store").finish()
    }
}
