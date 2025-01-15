use crate::engines::api::StoreEngine;
use qmdb::{config::Config, test_helper::SimpleTask, AdsCore, AdsWrap};
use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex},
};

const STATE_TRIE_NODES_TABLE: &str = "StateTrieNodes";
const BLOCK_NUMBERS_TABLE: &str = "BlockNumbers";
const BLOCK_TOTAL_DIFFICULTIES_TABLE: &str = "BlockTotalDifficulties";
const HEADERS_TABLE: &str = "Headers";
const BLOCK_BODIES_TABLE: &str = "BlockBodies";
const ACCOUNT_CODES_TABLE: &str = "AccountCodes";
const RECEIPTS_TABLE: &str = "Receipts";
const CANONICAL_BLOCK_HASHES_TABLE: &str = "CanonicalBlockHashes";
const STORAGE_TRIE_NODES_TABLE: &str = "StorageTrieNodes";
const CHAIN_DATA_TABLE: &str = "ChainData";
const PAYLOADS_TABLE: &str = "Payloads";
const PENDING_BLOCKS_TABLE: &str = "PendingBlocks";
const TRANSACTION_LOCATIONS_TABLE: &str = "TransactionLocations";

const TABLES: [&str; 13] = [
    STATE_TRIE_NODES_TABLE,
    BLOCK_NUMBERS_TABLE,
    BLOCK_TOTAL_DIFFICULTIES_TABLE,
    HEADERS_TABLE,
    BLOCK_BODIES_TABLE,
    ACCOUNT_CODES_TABLE,
    RECEIPTS_TABLE,
    CANONICAL_BLOCK_HASHES_TABLE,
    STORAGE_TRIE_NODES_TABLE,
    CHAIN_DATA_TABLE,
    PAYLOADS_TABLE,
    PENDING_BLOCKS_TABLE,
    TRANSACTION_LOCATIONS_TABLE,
];

pub struct Store {
    db: Arc<Mutex<HashMap<String, AdsWrap<SimpleTask>>>>,
}

impl Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("db", &"place holder".to_owned())
            .finish()
    }
}

impl Store {
    pub fn new() -> Self {
        let db: HashMap<String, AdsWrap<SimpleTask>> = TABLES
            .into_iter()
            .map(|table_name| {
                let config = Config::from_dir(table_name);
                AdsCore::init_dir(&config);
                let db: AdsWrap<SimpleTask> = AdsWrap::new(&config);
                (table_name.to_owned(), db)
            })
            .collect();
        Self {
            db: Arc::new(Mutex::new(db)),
        }
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_header: ethrex_core::types::BlockHeader,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<ethrex_core::types::BlockHash>,
        block_headers: Vec<ethrex_core::types::BlockHeader>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_header(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_body(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_body: ethrex_core::types::BlockBody,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_body(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        todo!()
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        todo!()
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        todo!()
    }

    fn add_pending_block(
        &self,
        block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_pending_block(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::Block>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_number(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_number(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_total_difficulty(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_total_difficulty: ethrex_core::U256,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::U256>, crate::error::StoreError> {
        todo!()
    }

    fn add_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
        block_number: ethrex_core::types::BlockNumber,
        block_hash: ethrex_core::types::BlockHash,
        index: ethrex_core::types::Index,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(
            ethrex_core::H256,
            ethrex_core::types::BlockNumber,
            ethrex_core::types::BlockHash,
            ethrex_core::types::Index,
        )>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
    ) -> Result<
        Option<(
            ethrex_core::types::BlockNumber,
            ethrex_core::types::BlockHash,
            ethrex_core::types::Index,
        )>,
        crate::error::StoreError,
    > {
        todo!()
    }

    fn add_receipt(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        index: ethrex_core::types::Index,
        receipt: ethrex_core::types::Receipt,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_receipts(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        receipts: Vec<ethrex_core::types::Receipt>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipt(
        &self,
        block_number: ethrex_core::types::BlockNumber,
        index: ethrex_core::types::Index,
    ) -> Result<Option<ethrex_core::types::Receipt>, crate::error::StoreError> {
        todo!()
    }

    fn add_account_code(
        &self,
        code_hash: ethrex_core::H256,
        code: bytes::Bytes,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_account_code(
        &self,
        code_hash: ethrex_core::H256,
    ) -> Result<Option<bytes::Bytes>, crate::error::StoreError> {
        todo!()
    }

    fn get_canonical_block_hash(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHash>, crate::error::StoreError> {
        todo!()
    }

    fn set_chain_config(
        &self,
        chain_config: &ethrex_core::types::ChainConfig,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_chain_config(
        &self,
    ) -> Result<ethrex_core::types::ChainConfig, crate::error::StoreError> {
        todo!()
    }

    fn update_earliest_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_earliest_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_finalized_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_finalized_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_safe_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_safe_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_latest_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_latest_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: ethrex_core::U256,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_latest_total_difficulty(
        &self,
    ) -> Result<Option<ethrex_core::U256>, crate::error::StoreError> {
        todo!()
    }

    fn update_pending_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_pending_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn open_storage_trie(
        &self,
        hashed_address: ethrex_core::H256,
        storage_root: ethrex_core::H256,
    ) -> ethrex_trie::Trie {
        todo!()
    }

    fn open_state_trie(&self, state_root: ethrex_core::H256) -> ethrex_trie::Trie {
        todo!()
    }

    fn set_canonical_block(
        &self,
        number: ethrex_core::types::BlockNumber,
        hash: ethrex_core::types::BlockHash,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn unset_canonical_block(
        &self,
        number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_payload(
        &self,
        payload_id: u64,
        block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_payload(
        &self,
        payload_id: u64,
    ) -> Result<
        Option<(
            ethrex_core::types::Block,
            ethrex_core::U256,
            ethrex_core::types::BlobsBundle,
            bool,
        )>,
        crate::error::StoreError,
    > {
        todo!()
    }

    fn update_payload(
        &self,
        payload_id: u64,
        block: ethrex_core::types::Block,
        block_value: ethrex_core::U256,
        blobs_bundle: ethrex_core::types::BlobsBundle,
        completed: bool,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipts_for_block(
        &self,
        block_hash: &ethrex_core::types::BlockHash,
    ) -> Result<Vec<ethrex_core::types::Receipt>, crate::error::StoreError> {
        todo!()
    }
}
