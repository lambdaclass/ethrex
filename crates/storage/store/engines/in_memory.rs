use crate::error::StoreError;
use bytes::Bytes;
use ethereum_rust_core::types::{
    Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
};
use ethereum_rust_trie::{InMemoryTrieDB, Trie};
use ethereum_types::{Address, H256, U256};
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
    storage_trie_nodes: HashMap<Address, NodeMap>,
    // TODO (#307): Remove TotalDifficulty.
    block_total_difficulties: HashMap<BlockHash, U256>,
    // Stores local blocks by payload id
    payloads: HashMap<u64, Block>,
    pending_blocks: HashMap<BlockHash, Block>,
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

    fn open_storage_trie(&self, address: Address, storage_root: H256) -> Trie {
        let mut store = self.inner();
        let trie_backend = store.storage_trie_nodes.entry(address).or_default();
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
        self.inner().payloads.insert(payload_id, block);
        Ok(())
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<Block>, StoreError> {
        Ok(self.inner().payloads.get(&payload_id).cloned())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("In Memory Store").finish()
    }
}
