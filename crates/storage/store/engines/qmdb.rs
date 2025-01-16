use crate::{engines::api::StoreEngine, error::StoreError};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use qmdb::{
    config::Config, seqads::SeqAdsWrap, test_helper::SimpleTask, utils::hasher, AdsCore, ADS,
};
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
    db: Arc<Mutex<HashMap<String, SeqAdsWrap<SimpleTask>>>>,
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
        let db: HashMap<String, SeqAdsWrap<SimpleTask>> = TABLES
            .into_iter()
            .map(|table_name| {
                let config = Config::from_dir(table_name);
                AdsCore::init_dir(&config);
                let db: SeqAdsWrap<SimpleTask> = SeqAdsWrap::new(&config);
                (table_name.to_owned(), db)
            })
            .collect();
        Self {
            db: Arc::new(Mutex::new(db)),
        }
    }

    pub fn read<K, V>(&self, key: K, table: &str) -> Result<Option<V>, crate::error::StoreError>
    where
        K: RLPEncode,
        V: RLPDecode,
    {
        let height = 0;
        let Some(value) = self._read(
            table,
            height,
            &key.encode_to_vec(),
            std::mem::size_of::<V>(),
        )?
        else {
            return Ok(None);
        };

        V::decode(&value)
            .map_err(crate::error::StoreError::from)
            .map(Some)
    }

    fn get_block_hash_by_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHash>, crate::error::StoreError> {
        Ok(self
            ._read(
                CANONICAL_BLOCK_HASHES_TABLE,
                block_number.try_into().unwrap(),
                &block_number.encode_to_vec(),
                std::mem::size_of::<ethrex_core::types::BlockHash>(),
            )?
            .map(|b| ethrex_core::types::BlockHash::from_slice(&b)))
    }

    fn _read(
        &self,
        table: &str,
        height: i64,
        key: &[u8],
        value_size: usize,
    ) -> Result<Option<Vec<u8>>, crate::error::StoreError> {
        let mut buf = vec![0; value_size];
        Ok(self
            .db
            .lock()
            .map_err(|err| StoreError::Custom(format!("Could not lock db: {err}")))?
            .get(table)
            .filter(|table_ads| {
                let key_hash = hasher::hash(key);
                let (_size, found_it) = table_ads.read_entry(height, &key_hash, key, &mut buf);
                found_it
            })
            .is_some()
            .then_some(buf))
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        _block_hash: ethrex_core::types::BlockHash,
        _block_header: ethrex_core::types::BlockHeader,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_block_headers(
        &self,
        _block_hashes: Vec<ethrex_core::types::BlockHash>,
        _block_headers: Vec<ethrex_core::types::BlockHeader>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_header(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        self.read(block_number, HEADERS_TABLE)
    }

    fn add_block_body(
        &self,
        _block_hash: ethrex_core::types::BlockHash,
        _block_body: ethrex_core::types::BlockBody,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_body(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        self.read(block_number, BLOCK_BODIES_TABLE)
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        self.read(block_hash, BLOCK_BODIES_TABLE)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        self.read(block_hash, HEADERS_TABLE)
    }

    fn add_pending_block(
        &self,
        _block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_pending_block(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::Block>, crate::error::StoreError> {
        self.read(block_hash, PENDING_BLOCKS_TABLE)
    }

    fn add_block_number(
        &self,
        _block_hash: ethrex_core::types::BlockHash,
        _block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_number(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        self.read(block_hash, BLOCK_NUMBERS_TABLE)
    }

    fn add_block_total_difficulty(
        &self,
        _block_hash: ethrex_core::types::BlockHash,
        _block_total_difficulty: ethrex_core::U256,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::U256>, crate::error::StoreError> {
        self.read(block_hash, BLOCK_TOTAL_DIFFICULTIES_TABLE)
    }

    fn add_transaction_location(
        &self,
        _transaction_hash: ethrex_core::H256,
        _block_number: ethrex_core::types::BlockNumber,
        _block_hash: ethrex_core::types::BlockHash,
        _index: ethrex_core::types::Index,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_transaction_locations(
        &self,
        _locations: Vec<(
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
        _transaction_hash: ethrex_core::H256,
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
        _block_hash: ethrex_core::types::BlockHash,
        _index: ethrex_core::types::Index,
        _receipt: ethrex_core::types::Receipt,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_receipts(
        &self,
        _block_hash: ethrex_core::types::BlockHash,
        _receipts: Vec<ethrex_core::types::Receipt>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipt(
        &self,
        block_number: ethrex_core::types::BlockNumber,
        index: ethrex_core::types::Index,
    ) -> Result<Option<ethrex_core::types::Receipt>, crate::error::StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };

        self.read((block_hash, index), RECEIPTS_TABLE)
    }

    fn add_account_code(
        &self,
        _code_hash: ethrex_core::H256,
        _code: bytes::Bytes,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_account_code(
        &self,
        code_hash: ethrex_core::H256,
    ) -> Result<Option<bytes::Bytes>, crate::error::StoreError> {
        self.read(code_hash, ACCOUNT_CODES_TABLE)
    }

    fn get_canonical_block_hash(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHash>, crate::error::StoreError> {
        self.read(block_number, CANONICAL_BLOCK_HASHES_TABLE)
    }

    fn set_chain_config(
        &self,
        _chain_config: &ethrex_core::types::ChainConfig,
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
        _block_number: ethrex_core::types::BlockNumber,
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
        _block_number: ethrex_core::types::BlockNumber,
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
        _block_number: ethrex_core::types::BlockNumber,
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
        _block_number: ethrex_core::types::BlockNumber,
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
        _latest_total_difficulty: ethrex_core::U256,
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
        _block_number: ethrex_core::types::BlockNumber,
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
        _hashed_address: ethrex_core::H256,
        _storage_root: ethrex_core::H256,
    ) -> ethrex_trie::Trie {
        todo!()
    }

    fn open_state_trie(&self, _state_root: ethrex_core::H256) -> ethrex_trie::Trie {
        todo!()
    }

    fn set_canonical_block(
        &self,
        _number: ethrex_core::types::BlockNumber,
        _hash: ethrex_core::types::BlockHash,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn unset_canonical_block(
        &self,
        _number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_payload(
        &self,
        _payload_id: u64,
        _block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_payload(
        &self,
        _payload_id: u64,
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
        _payload_id: u64,
        _block: ethrex_core::types::Block,
        _block_value: ethrex_core::U256,
        _blobs_bundle: ethrex_core::types::BlobsBundle,
        _completed: bool,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipts_for_block(
        &self,
        block_hash: &ethrex_core::types::BlockHash,
    ) -> Result<Vec<ethrex_core::types::Receipt>, crate::error::StoreError> {
        self.read(*block_hash, RECEIPTS_TABLE)
            .map(Option::unwrap_or_default)
    }
}
