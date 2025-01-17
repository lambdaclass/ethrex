use crate::{
    engines::{api::StoreEngine, utils::ChainDataIndex},
    error::StoreError,
};
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
    config::Config,
    def::{IN_BLOCK_IDX_BITS, OP_CREATE, OP_DELETE, OP_WRITE},
    seqads::SeqAdsWrap,
    test_helper::SimpleTask,
    utils::{byte0_to_shard_id, changeset::ChangeSet, hasher},
    AdsCore, ADS,
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

    fn read<K, V>(&self, key: K, table: &str) -> Result<Option<V>, StoreError>
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

    fn get_chain_data<V>(&self, key: impl Into<u8>) -> Result<Option<V>, StoreError>
    where
        V: RLPDecode,
    {
        let Some(bytes) = self.read::<_, Vec<u8>>(key.into(), CHAIN_DATA_TABLE)? else {
            return Ok(None);
        };
        V::decode(&bytes).map_err(StoreError::from).map(Some)
    }

    fn get_block_hash_by_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            ._read(
                CANONICAL_BLOCK_HASHES_TABLE,
                block_number.try_into().map_err(|err| {
                    StoreError::Custom(format!("Could not convert block number: {err}"))
                })?,
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

    fn write<K, V>(&self, key: K, value: V, table: &str, operation: u8) -> Result<(), StoreError>
    where
        K: RLPEncode,
        V: RLPEncode,
    {
        let mut change_set = ChangeSet::new();
        self._add_op_to_change_set(&mut change_set, key, value, operation);
        change_set.sort();
        self._write(table, &vec![change_set])
    }

    fn write_batch<K, V>(
        &self,
        key_value_tuples: Vec<(K, V)>,
        table: &str,
        operation: u8,
    ) -> Result<(), StoreError>
    where
        K: RLPEncode,
        V: RLPEncode,
    {
        let mut change_set = ChangeSet::new();
        for (key, value) in key_value_tuples {
            self._add_op_to_change_set(&mut change_set, key, value, operation);
        }
        change_set.sort();
        self._write(table, &vec![change_set])
    }

    fn _add_op_to_change_set<K, V>(&self, change_set: &mut ChangeSet, key: K, value: V, op: u8)
    where
        K: RLPEncode,
        V: RLPEncode,
    {
        let encoded_key = key.encode_to_vec();
        let encoded_value = value.encode_to_vec();

        let key_hash = hasher::hash(&encoded_key);
        let shard_id = byte0_to_shard_id(key_hash[0]) as u8;
        change_set.add_op(op, shard_id, &key_hash, &encoded_key, &encoded_value, None);
    }

    fn _write(&self, table: &str, change_sets: &Vec<ChangeSet>) -> Result<(), StoreError> {
        self.db
            .lock()
            .map_err(|err| StoreError::Custom(format!("Could not lock db: {err}")))
            .and_then(|db| {
                db.get(table)
                    .map(|table_ads| {
                        let task_id = 1 << IN_BLOCK_IDX_BITS;
                        table_ads.commit_tx(task_id, change_sets);
                    })
                    .ok_or(StoreError::InternalError(format!(
                        "Table {table} not found"
                    )))
            })
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write(block_hash, block_header, HEADERS_TABLE, OP_CREATE)
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        self.write_batch(
            block_hashes.into_iter().zip(block_headers).collect(),
            HEADERS_TABLE,
            OP_CREATE,
        )
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
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write(block_hash, block_body, BLOCK_BODIES_TABLE, OP_CREATE)
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

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write(block.hash(), block, PENDING_BLOCKS_TABLE, OP_CREATE)
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        self.read(block_hash, PENDING_BLOCKS_TABLE)
    }

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(block_hash, block_number, BLOCK_NUMBERS_TABLE, OP_CREATE)
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        self.read(block_hash, BLOCK_NUMBERS_TABLE)
    }

    fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        self.write(
            block_hash,
            block_total_difficulty,
            BLOCK_TOTAL_DIFFICULTIES_TABLE,
            OP_CREATE,
        )
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
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.write((block_hash, index), receipt, RECEIPTS_TABLE, OP_CREATE)
    }

    fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        self.write_batch(
            receipts
                .into_iter()
                .enumerate()
                .map(|(index, receipt)| ((block_hash, index), receipt))
                .collect(),
            RECEIPTS_TABLE,
            OP_CREATE,
        )
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

    fn add_account_code(&self, code_hash: H256, code: bytes::Bytes) -> Result<(), StoreError> {
        self.write(code_hash, code, ACCOUNT_CODES_TABLE, OP_CREATE)
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

    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::ChainConfig as u8,
            serde_json::to_string(chain_config).map_err(|_err| StoreError::DecodeError)?,
            CHAIN_DATA_TABLE,
            OP_CREATE, // FIXME: Create or write?
        )
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        let bytes = self
            .read::<_, Vec<u8>>(ChainDataIndex::ChainConfig as u8, CHAIN_DATA_TABLE)?
            .ok_or(StoreError::Custom("Chain config not found".to_string()))?;
        let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
        serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::EarliestBlockNumber as u8,
            block_number,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.get_chain_data(ChainDataIndex::EarliestBlockNumber as u8)
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::FinalizedBlockNumber as u8,
            block_number,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.get_chain_data(ChainDataIndex::FinalizedBlockNumber as u8)
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::SafeBlockNumber as u8,
            block_number,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.get_chain_data(ChainDataIndex::SafeBlockNumber as u8)
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::LatestBlockNumber as u8,
            block_number,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.get_chain_data(ChainDataIndex::LatestBlockNumber as u8)
    }

    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: U256,
    ) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::LatestTotalDifficulty as u8,
            latest_total_difficulty,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_latest_total_difficulty(&self) -> Result<Option<U256>, StoreError> {
        self.get_chain_data(ChainDataIndex::LatestTotalDifficulty as u8)
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            ChainDataIndex::PendingBlockNumber as u8,
            block_number,
            CHAIN_DATA_TABLE,
            OP_WRITE,
        )
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        self.get_chain_data(ChainDataIndex::PendingBlockNumber as u8)
    }

    fn open_storage_trie(&self, _hashed_address: H256, _storage_root: H256) -> Trie {
        todo!()
    }

    fn open_state_trie(&self, _state_root: H256) -> Trie {
        todo!()
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        self.write(number, hash, CANONICAL_BLOCK_HASHES_TABLE, OP_CREATE)
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.write(number, (), CANONICAL_BLOCK_HASHES_TABLE, OP_DELETE)
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write(
            payload_id,
            (block, U256::zero(), BlobsBundle::empty(), false),
            PAYLOADS_TABLE,
            OP_CREATE,
        )
    }

    fn get_payload(
        &self,
        payload_id: u64,
    ) -> Result<Option<(Block, U256, BlobsBundle, bool)>, StoreError> {
        self.read(payload_id, PAYLOADS_TABLE)
    }

    fn update_payload(
        &self,
        payload_id: u64,
        block: Block,
        block_value: U256,
        blobs_bundle: BlobsBundle,
        completed: bool,
    ) -> Result<(), StoreError> {
        self.write(
            payload_id,
            (block, block_value, blobs_bundle, completed),
            PAYLOADS_TABLE,
            OP_WRITE,
        )
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        self.read(*block_hash, RECEIPTS_TABLE)
            .map(Option::unwrap_or_default)
    }
}
