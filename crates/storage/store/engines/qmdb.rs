use crate::{engines::api::StoreEngine, error::StoreError};
use ethrex_core::{
    types::{
        BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
        Receipt,
    },
    H256, U256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::Trie;
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

    pub fn read<K, V>(&self, key: K, table: &str) -> Result<Option<V>, StoreError>
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

        V::decode(&value).map_err(StoreError::from).map(Some)
    }

    fn get_block_hash_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            ._read(
                CANONICAL_BLOCK_HASHES_TABLE,
                block_number.try_into().unwrap(),
                &block_number.encode_to_vec(),
                std::mem::size_of::<BlockHash>(),
            )?
            .map(|b| BlockHash::from_slice(&b)))
    }

    fn _read(
        &self,
        table: &str,
        height: i64,
        key: &[u8],
        value_size: usize,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.db
            .lock()
            .map_err(|err| StoreError::Custom(format!("Could not lock db: {err}")))
            .map(|db_lock| {
                let mut buf = vec![0; value_size];
                db_lock
                    .get(table)
                    .filter(|table_ads| {
                        let key_hash = hasher::hash(key);
                        let (_size, found_it) =
                            table_ads.read_entry(height, &key_hash, key, &mut buf);
                        found_it
                    })
                    .is_some()
                    .then_some(buf)
            })
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        _block_hash: BlockHash,
        _block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn add_block_headers(
        &self,
        _block_hashes: Vec<BlockHash>,
        _block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };
        self.read(block_hash, HEADERS_TABLE)
    }

    fn add_block_body(
        &self,
        _block_hash: BlockHash,
        _block_body: BlockBody,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_body(&self, block_number: BlockNumber) -> Result<Option<BlockBody>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };
        self.read(block_hash, BLOCK_BODIES_TABLE)
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.read(block_hash, BLOCK_BODIES_TABLE)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.read(block_hash, HEADERS_TABLE)
    }

    fn add_pending_block(&self, _block: Block) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        self.read(block_hash, PENDING_BLOCKS_TABLE)
    }

    fn add_block_number(
        &self,
        _block_hash: BlockHash,
        _block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        self.read(block_hash, BLOCK_NUMBERS_TABLE)
    }

    fn add_block_total_difficulty(
        &self,
        _block_hash: BlockHash,
        _block_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<U256>, StoreError> {
        self.read(block_hash, BLOCK_TOTAL_DIFFICULTIES_TABLE)
    }

    fn add_transaction_location(
        &self,
        _transaction_hash: H256,
        _block_number: BlockNumber,
        _block_hash: BlockHash,
        _index: Index,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn add_transaction_locations(
        &self,
        _locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_transaction_location(
        &self,
        _transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        todo!()
    }

    fn add_receipt(
        &self,
        _block_hash: BlockHash,
        _index: Index,
        _receipt: Receipt,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn add_receipts(
        &self,
        _block_hash: BlockHash,
        _receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };
        self.read((block_hash, index), RECEIPTS_TABLE)
    }

    fn add_account_code(&self, _code_hash: H256, _code: bytes::Bytes) -> Result<(), StoreError> {
        todo!()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<bytes::Bytes>, StoreError> {
        self.read(code_hash, ACCOUNT_CODES_TABLE)
    }

    fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read(block_number, CANONICAL_BLOCK_HASHES_TABLE)
    }

    fn set_chain_config(&self, _chain_config: &ChainConfig) -> Result<(), StoreError> {
        todo!()
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        todo!()
    }

    fn update_earliest_block_number(&self, _block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_finalized_block_number(&self, _block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_safe_block_number(&self, _block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_latest_block_number(&self, _block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_latest_total_difficulty(
        &self,
        _latest_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError> {
        todo!()
    }

    fn update_pending_block_number(&self, _block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn open_storage_trie(&self, _hashed_address: H256, _storage_root: H256) -> Trie {
        todo!()
    }

    fn open_state_trie(&self, _state_root: H256) -> Trie {
        todo!()
    }

    fn set_canonical_block(
        &self,
        _number: BlockNumber,
        _hash: BlockHash,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn unset_canonical_block(&self, _number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn add_payload(&self, _payload_id: u64, _block: Block) -> Result<(), StoreError> {
        todo!()
    }

    fn get_payload(
        &self,
        _payload_id: u64,
    ) -> Result<Option<(Block, U256, BlobsBundle, bool)>, StoreError> {
        todo!()
    }

    fn update_payload(
        &self,
        _payload_id: u64,
        _block: Block,
        _block_value: U256,
        _blobs_bundle: BlobsBundle,
        _completed: bool,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        self.read(*block_hash, RECEIPTS_TABLE)
            .map(Option::unwrap_or_default)
    }
}
