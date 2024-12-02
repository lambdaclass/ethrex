use std::{borrow::Borrow, panic::RefUnwindSafe, sync::Arc};

use ethrex_core::types::BlockBody;
use ethrex_core::U256;
use ethrex_core::{
    types::{Block, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt},
    H256,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{
    db::{redb::RedBTrie, redb_multitable::RedBMultiTableTrieDB},
    Trie,
};
use redb::{AccessGuard, Database, Key, MultimapTableDefinition, TableDefinition, TypeName, Value};

use crate::rlp::{BlockRLP, BlockTotalDifficultyRLP, Rlp, TransactionHashRLP};
use crate::{
    error::StoreError,
    rlp::{
        AccountCodeHashRLP, AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP, ReceiptRLP,
        TupleRLP,
    },
};

use super::{api::StoreEngine, utils::ChainDataIndex};

const STATE_TRIE_NODES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("StateTrieNodes");
const BLOCK_NUMBERS_TABLE: TableDefinition<BlockHashRLP, BlockNumber> =
    TableDefinition::new("BlockNumbers");
const BLOCK_TOTAL_DIFFICULTIES_TABLE: TableDefinition<BlockHashRLP, BlockTotalDifficultyRLP> =
    TableDefinition::new("BlockTotalDifficulties");
const HEADERS_TABLE: TableDefinition<BlockHashRLP, BlockHeaderRLP> =
    TableDefinition::new("Headers");
const BLOCK_BODIES_TABLE: TableDefinition<BlockHashRLP, BlockBodyRLP> =
    TableDefinition::new("BlockBodies");
const ACCOUNT_CODES_TABLE: TableDefinition<AccountCodeHashRLP, AccountCodeRLP> =
    TableDefinition::new("AccountCodes");
const RECEIPTS_TABLE: TableDefinition<TupleRLP<BlockHash, Index>, ReceiptRLP> =
    TableDefinition::new("Receipts");
const CANONICAL_BLOCK_HASHES_TABLE: TableDefinition<BlockNumber, BlockHashRLP> =
    TableDefinition::new("CanonicalBlockHashes");
pub const STORAGE_TRIE_NODES_TABLE: MultimapTableDefinition<([u8; 32], [u8; 33]), &[u8]> =
    MultimapTableDefinition::new("StorageTrieNodes");
const CHAIN_DATA_TABLE: TableDefinition<ChainDataIndex, Vec<u8>> =
    TableDefinition::new("ChainData");
const PAYLOADS_TABLE: TableDefinition<BlockNumber, BlockRLP> = TableDefinition::new("Payloads");
const PENDING_BLOCKS_TABLE: TableDefinition<BlockHashRLP, BlockRLP> =
    TableDefinition::new("PendingBlocks");
const TRANSACTION_LOCATIONS_TABLE: MultimapTableDefinition<
    TransactionHashRLP,
    Rlp<(BlockNumber, BlockHash, Index)>,
> = MultimapTableDefinition::new("TransactionLocations");

#[derive(Debug)]
pub struct RedBStore {
    db: Arc<Database>,
}

impl RefUnwindSafe for RedBStore {}
impl RedBStore {
    pub fn new() -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db()?),
        })
    }

    // Helper method to write into a redb table
    fn write<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
        value: impl Borrow<V::SelfType<'v>>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Value + 'static,
    {
        let write_txn = self.db.begin_write()?;
        write_txn.open_table(table)?.insert(key, value)?;
        write_txn.commit()?;

        Ok(())
    }

    // Helper method to write into a redb table
    fn write_to_multi<'k, 'v, 'a, K, V>(
        &self,
        table: MultimapTableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
        value: impl Borrow<V::SelfType<'v>>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Key + 'static,
    {
        let write_txn = self.db.begin_write()?;
        write_txn.open_multimap_table(table)?.insert(key, value)?;
        write_txn.commit()?;

        Ok(())
    }

    // Helper method to write into a redb table
    fn write_batch<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key_values: Vec<(impl Borrow<K::SelfType<'k>>, impl Borrow<V::SelfType<'v>>)>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Value + 'static,
    {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(table)?;
            for (key, value) in key_values {
                table.insert(key, value)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    }

    // Helper method to write into a redb table
    fn write_to_multi_batch<'k, 'v, 'a, K, V>(
        &self,
        table: MultimapTableDefinition<'a, K, V>,
        key_values: Vec<(impl Borrow<K::SelfType<'k>>, impl Borrow<V::SelfType<'v>>)>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Key + 'static,
    {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_multimap_table(table)?;
            for (key, value) in key_values {
                table.insert(key, value)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    }

    // Helper method to read from a redb table
    fn read<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
    ) -> Result<Option<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + 'static,
        V: Value,
    {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(table)?;
        let result = table.get(key)?;

        Ok(result)
    }

    // Helper method to delete from a redb table
    fn delete<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Value,
    {
        let write_txn = self.db.begin_write()?;
        write_txn.open_table(table)?.remove(key)?;
        write_txn.commit()?;

        Ok(())
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            .read(CANONICAL_BLOCK_HASHES_TABLE, number)?
            .map(|a| a.value().to()))
    }
}

impl StoreEngine for RedBStore {
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write(
            HEADERS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
            <BlockHeader as Into<BlockHeaderRLP>>::into(block_header),
        )
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self
                .read(HEADERS_TABLE, <H256 as Into<BlockHashRLP>>::into(hash))?
                .map(|b| b.value().to()))
        } else {
            Ok(None)
        }
    }

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write(
            BLOCK_BODIES_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
            <BlockBody as Into<BlockBodyRLP>>::into(block_body),
        )
    }

    fn get_block_body(&self, block_number: BlockNumber) -> Result<Option<BlockBody>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.get_block_body_by_hash(hash)
        } else {
            Ok(None)
        }
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        Ok(self
            .read(
                BLOCK_BODIES_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value().to()))
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        Ok(self
            .read(
                HEADERS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value().to()))
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write(
            PENDING_BLOCKS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block.header.compute_block_hash()),
            <Block as Into<BlockRLP>>::into(block),
        )
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        Ok(self
            .read(
                PENDING_BLOCKS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value().to()))
    }

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            BLOCK_NUMBERS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
            block_number,
        )
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self
            .read(
                BLOCK_NUMBERS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value()))
    }

    fn add_block_total_difficulty(
        &self,
        block_hash: BlockHash,
        block_total_difficulty: ethrex_core::U256,
    ) -> Result<(), StoreError> {
        // self.write::<BlockTotalDifficulties>(block_hash.into(), block_total_difficulty.into())
        self.write(
            BLOCK_TOTAL_DIFFICULTIES_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
            <U256 as Into<Rlp<U256>>>::into(block_total_difficulty),
        )
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<ethrex_core::U256>, StoreError> {
        Ok(self
            .read(
                BLOCK_TOTAL_DIFFICULTIES_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value().to()))
    }

    fn add_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.write_to_multi(
            TRANSACTION_LOCATIONS_TABLE,
            <H256 as Into<TransactionHashRLP>>::into(transaction_hash),
            <(u64, H256, u64) as Into<Rlp<(BlockNumber, BlockHash, Index)>>>::into((
                block_number,
                block_hash,
                index,
            )),
        )
    }

    fn get_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;

        Ok(table
            .get(<H256 as Into<TransactionHashRLP>>::into(transaction_hash))?
            .map_while(|res| res.ok().map(|t| t.value().to()))
            .find(|(number, hash, _index)| {
                self.get_block_hash_by_block_number(*number)
                    .is_ok_and(|o| o == Some(*hash))
            }))
    }

    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        self.write(
            RECEIPTS_TABLE,
            <(H256, u64) as Into<TupleRLP<BlockHash, Index>>>::into((block_hash, index)),
            <Receipt as Into<ReceiptRLP>>::into(receipt),
        )
    }

    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self
                .read(
                    RECEIPTS_TABLE,
                    <(H256, u64) as Into<TupleRLP<BlockHash, Index>>>::into((hash, index)),
                )?
                .map(|b| b.value().to()))
        } else {
            Ok(None)
        }
    }

    fn add_account_code(
        &self,
        code_hash: ethrex_core::H256,
        code: bytes::Bytes,
    ) -> Result<(), StoreError> {
        self.write(
            ACCOUNT_CODES_TABLE,
            <H256 as Into<AccountCodeHashRLP>>::into(code_hash),
            <bytes::Bytes as Into<AccountCodeRLP>>::into(code),
        )
    }

    fn get_account_code(
        &self,
        code_hash: ethrex_core::H256,
    ) -> Result<Option<bytes::Bytes>, StoreError> {
        Ok(self
            .read(
                ACCOUNT_CODES_TABLE,
                <H256 as Into<AccountCodeHashRLP>>::into(code_hash),
            )?
            .map(|b| b.value().to()))
    }

    fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read(CANONICAL_BLOCK_HASHES_TABLE, block_number)
            .map(|o| o.map(|hash_rlp| hash_rlp.value().to()))
    }

    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes.value()).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::EarliestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::FinalizedBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::SafeBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::LatestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: ethrex_core::U256,
    ) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::LatestTotalDifficulty,
            latest_total_difficulty.encode_to_vec(),
        )
    }

    fn get_latest_total_difficulty(&self) -> Result<Option<ethrex_core::U256>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::LatestTotalDifficulty)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read(CHAIN_DATA_TABLE, ChainDataIndex::PendingBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(
        &self,
        hashed_address: ethrex_core::H256,
        storage_root: ethrex_core::H256,
    ) -> ethrex_trie::Trie {
        let db = Box::new(RedBMultiTableTrieDB::new(self.db.clone(), hashed_address.0));
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: ethrex_core::H256) -> ethrex_trie::Trie {
        let db = Box::new(RedBTrie::new(self.db.clone()));
        Trie::open(db, state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        self.write(
            CANONICAL_BLOCK_HASHES_TABLE,
            number,
            <H256 as Into<BlockHashRLP>>::into(hash),
        )
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.delete(CANONICAL_BLOCK_HASHES_TABLE, number)
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write(
            PAYLOADS_TABLE,
            payload_id,
            <Block as Into<BlockRLP>>::into(block),
        )
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<Block>, StoreError> {
        Ok(self
            .read(PAYLOADS_TABLE, payload_id)?
            .map(|b| b.value().to()))
    }

    fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let key_values = receipts
            .into_iter()
            .enumerate()
            .map(|(index, receipt)| {
                (
                    <(H256, u64) as Into<TupleRLP<BlockHash, Index>>>::into((
                        block_hash,
                        index as u64,
                    )),
                    <Receipt as Into<ReceiptRLP>>::into(receipt),
                )
            })
            .collect();
        self.write_batch(RECEIPTS_TABLE, key_values)
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (
                    <H256 as Into<TransactionHashRLP>>::into(tx_hash),
                    <(u64, H256, u64) as Into<Rlp<(BlockNumber, BlockHash, Index)>>>::into((
                        block_number,
                        block_hash,
                        index,
                    )),
                )
            })
            .collect();

        self.write_to_multi_batch(TRANSACTION_LOCATIONS_TABLE, key_values)?;

        Ok(())
    }
}

impl redb::Value for ChainDataIndex {
    type SelfType<'a> = ChainDataIndex
    where
        Self: 'a;

    type AsBytes<'a> = [u8; 1]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        data[0].into()
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        [*value as u8]
    }

    fn type_name() -> redb::TypeName {
        TypeName::new("ChainDataIndex")
    }
}

impl redb::Key for ChainDataIndex {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

pub fn init_db() -> Result<Database, StoreError> {
    let db = Database::create("ethrex.redb")?;

    let table_creation_txn = db.begin_write()?;
    table_creation_txn.open_table(STATE_TRIE_NODES_TABLE)?;
    table_creation_txn.open_table(BLOCK_NUMBERS_TABLE)?;
    table_creation_txn.open_table(BLOCK_TOTAL_DIFFICULTIES_TABLE)?;
    table_creation_txn.open_table(CANONICAL_BLOCK_HASHES_TABLE)?;
    table_creation_txn.open_table(RECEIPTS_TABLE)?;
    table_creation_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;
    table_creation_txn.open_table(CHAIN_DATA_TABLE)?;
    table_creation_txn.open_table(BLOCK_BODIES_TABLE)?;
    table_creation_txn.open_table(PAYLOADS_TABLE)?;
    table_creation_txn.open_table(PENDING_BLOCKS_TABLE)?;
    table_creation_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;
    table_creation_txn.commit()?;

    Ok(db)
}
