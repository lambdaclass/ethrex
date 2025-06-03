use std::{borrow::Borrow, collections::HashMap, panic::RefUnwindSafe, sync::Arc};

use crate::api::{StoreRoTx, StoreRwTx};
use crate::rlp::{
    AccountHashRLP, AccountStateRLP, BlockRLP, Rlp, TransactionHashRLP, TriePathsRLP,
};
use crate::store::MAX_SNAPSHOT_READS;
use crate::trie_db::{redb::RedBTrie, redb_multitable::RedBMultiTableTrieDB};
use crate::{
    error::StoreError,
    rlp::{
        AccountCodeHashRLP, AccountCodeRLP, BlockBodyRLP, BlockHashRLP, BlockHeaderRLP,
        PayloadBundleRLP, ReceiptRLP, TupleRLP,
    },
};
use ethrex_common::types::{AccountState, BlockBody};
use ethrex_common::{
    types::{
        payload::PayloadBundle, Block, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
        Receipt,
    },
    H256, U256,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie};
use redb::{
    AccessGuard, Database, Key, MultimapTableDefinition, ReadableTable, TableDefinition, TypeName,
    Value,
};

use crate::utils::SnapStateIndex;
use crate::{api::StoreEngine, utils::ChainDataIndex};

const STATE_TRIE_NODES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("StateTrieNodes");
const BLOCK_NUMBERS_TABLE: TableDefinition<BlockHashRLP, BlockNumber> =
    TableDefinition::new("BlockNumbers");
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
const INVALID_ANCESTORS_TABLE: TableDefinition<BlockHashRLP, BlockHashRLP> =
    TableDefinition::new("InvalidAncestors");
const PAYLOADS_TABLE: TableDefinition<BlockNumber, PayloadBundleRLP> =
    TableDefinition::new("Payloads");
const PENDING_BLOCKS_TABLE: TableDefinition<BlockHashRLP, BlockRLP> =
    TableDefinition::new("PendingBlocks");
const TRANSACTION_LOCATIONS_TABLE: MultimapTableDefinition<
    TransactionHashRLP,
    Rlp<(BlockNumber, BlockHash, Index)>,
> = MultimapTableDefinition::new("TransactionLocations");
const SNAP_STATE_TABLE: TableDefinition<SnapStateIndex, Vec<u8>> =
    TableDefinition::new("SnapState");
const STATE_SNAPSHOT_TABLE: TableDefinition<AccountHashRLP, AccountStateRLP> =
    TableDefinition::new("StateSnapshot");
const STORAGE_SNAPSHOT_TABLE: MultimapTableDefinition<AccountHashRLP, ([u8; 32], [u8; 32])> =
    MultimapTableDefinition::new("StorageSnapshotTable");
const STORAGE_HEAL_PATHS_TABLE: TableDefinition<AccountHashRLP, TriePathsRLP> =
    TableDefinition::new("StorageHealPaths");

#[derive(Debug)]
pub struct RedBStore {
    db: Arc<Database>,
}

trait TxKind {
    fn open_table<K: Key + 'static, V: Value + 'static>(
        &self,
        definition: TableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableTable<K, V>, redb::TableError>;
    fn open_multimap_table<K: Key + 'static, V: Key + 'static>(
        &self,
        definition: MultimapTableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableMultimapTable<K, V>, redb::TableError>;
}
impl TxKind for redb::WriteTransaction {
    fn open_table<K: Key + 'static, V: Value + 'static>(
        &self,
        definition: TableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableTable<K, V>, redb::TableError> {
        self.open_table(definition)
    }
    fn open_multimap_table<K: Key + 'static, V: Key + 'static>(
        &self,
        definition: MultimapTableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableMultimapTable<K, V>, redb::TableError> {
        self.open_multimap_table(definition)
    }
}
impl TxKind for redb::ReadTransaction {
    fn open_table<K: Key + 'static, V: Value + 'static>(
        &self,
        definition: TableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableTable<K, V>, redb::TableError> {
        self.open_table(definition)
    }
    fn open_multimap_table<K: Key + 'static, V: Key + 'static>(
        &self,
        definition: MultimapTableDefinition<'_, K, V>,
    ) -> Result<impl redb::ReadableMultimapTable<K, V>, redb::TableError> {
        self.open_multimap_table(definition)
    }
}

pub struct RedBTx<Kind: TxKind> {
    tx: Kind,
}
type RedBRwTx = RedBTrie<redb::WriteTransaction>;
type RedBRoTx = RedBTrie<redb::ReadTransaction>;

impl RefUnwindSafe for RedBStore {}
impl RedBStore {
    pub fn new() -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db()?),
        })
    }
}

impl RedBRwTx {
    // Helper method to write into a redb table
    async fn write<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
        value: V::SelfType<'v>,
    ) -> Result<(), StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send + 'static,
        K::SelfType<'k>: Send,
        V::SelfType<'v>: Send,
        'a: 'static,
        'k: 'static,
        'v: 'static,
    {
        self.tx.open_table(table)?.insert(key, value)?;
        Ok(())
    }

    // Helper method to write into a redb table
    async fn write_to_multi<'k, 'v, 'a, K, V>(
        &self,
        table: MultimapTableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
        value: V::SelfType<'v>,
    ) -> Result<(), StoreError>
    where
        K: Key + 'static,
        V: Key + 'static,
        K::SelfType<'k>: Send,
        V::SelfType<'v>: Send,
        MultimapTableDefinition<'a, K, V>: Send,
        'a: 'static,
        'k: 'static,
        'v: 'static,
    {
        self.tx.open_multimap_table(table)?.insert(key, value)?;
        Ok(())
    }

    // Helper method to write into a redb table
    async fn write_batch<'k, 'v, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key_values: Vec<(K::SelfType<'k>, V::SelfType<'v>)>,
    ) -> Result<(), StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send + 'static,
        K::SelfType<'k>: Send,
        V::SelfType<'v>: Send,
        TableDefinition<'a, K, V>: Send,
        'a: 'static,
        'k: 'static,
        'v: 'static,
    {
        let mut table = self.tx.open_table(table)?;
        for (key, value) in key_values {
            table.insert(key, value).map(|_| ())?; // Ignore return value
        }
        Ok(())
    }

    // Helper method to write into a redb table
    async fn write_to_multi_batch<'k, 'v, 'a, K, V>(
        &self,
        table: MultimapTableDefinition<'a, K, V>,
        key_values: Vec<(K::SelfType<'k>, V::SelfType<'v>)>,
    ) -> Result<(), StoreError>
    where
        K: Key + Send + 'static,
        V: Key + Send + 'static,
        K::SelfType<'k>: Send,
        V::SelfType<'v>: Send,
        MultimapTableDefinition<'a, K, V>: Send,
        'a: 'static,
        'k: 'static,
        'v: 'static,
    {
        let mut table = self.tx.open_multimap_table(table)?;
        for (key, value) in key_values {
            table.insert(key, value).map(|_| ())?; // Ignore return value
        }
        Ok(())
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
        self.tx
            .open_table(table)
            .map_err(|e| StoreError::Custom(format!("failed to open table: {e}")))?
            .remove(key)
            .map(|_| ()) // Ignore return value
            .map_err(|e| StoreError::Custom(format!("removal failed: {e}")))
    }
}

impl<Kind: TxKind> RedBTx<Kind> {
    // Helper method to read from a redb table
    async fn read<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: K::SelfType<'k>,
    ) -> Result<Option<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send,
        K::SelfType<'k>: Send,
        'a: 'static,
        'k: 'static,
    {
        self.tx.open_table(table)?.get(key)?
    }

    // Helper method to read in bulk from a redb table
    async fn read_bulk<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        keys: Vec<K::SelfType<'k>>,
    ) -> Result<Vec<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + Send + 'static,
        V: Value + Send,
        K::SelfType<'k>: Send,
        'a: 'static,
        'k: 'static,
    {
        let table = self
            .tx
            .open_table(table)
            .map_err(|e| StoreError::Custom(format!("failed to open table: {e}")))?;
        let mut result = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(value) = table
                .get(key)
                .map_err(|e| StoreError::Custom(format!("failed to retrieve key: {e}")))?
            {
                result.push(value);
            }
        }
        Ok(result)
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

#[async_trait::async_trait]
impl StoreEngine<'_> for RedBStore {
    fn begin_ro_tx(&self) -> Result<Arc<dyn StoreRoTx<'_>>, StoreError> {
        let tx = self.db.begin_read().map(|tx| RedBRoTx { tx })?;
        Ok(Arc::<dyn StoreRoTx<'_>>::new(tx))
    }
    fn begin_rw_tx(&self) -> Result<Arc<dyn StoreRwTx<'_>>, StoreError> {
        let tx = self.db.begin_write().map(|tx| RedBRwTx { tx })?;
        Ok(Arc::<dyn StoreRwTx<'_>>::new(tx))
    }
}

#[async_trait::async_trait]
impl StoreRoTx<'_> for RedBRoTx {
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

    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.get_block_body_by_hash(hash).await
        } else {
            Ok(None)
        }
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers = (from..=to).collect();
        let hashes = self
            .read_bulk(CANONICAL_BLOCK_HASHES_TABLE, numbers)
            .await?;
        let hashes: Vec<BlockHashRLP> = hashes.into_iter().map(|v| v.value()).collect();
        let blocks = self.read_bulk(BLOCK_BODIES_TABLE, hashes).await?;
        Ok(blocks.into_iter().map(|b| b.value().to()).collect())
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let hashes = hashes
            .into_iter()
            .map(<H256 as Into<BlockHashRLP>>::into)
            .collect();
        let blocks = self.read_bulk(BLOCK_BODIES_TABLE, hashes).await?;
        Ok(blocks.into_iter().map(|b| b.value().to()).collect())
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        Ok(self
            .read(
                BLOCK_BODIES_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )
            .await?
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

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        Ok(self
            .read(
                PENDING_BLOCKS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )
            .await?
            .map(|b| b.value().to()))
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self
            .read(
                BLOCK_NUMBERS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )
            .await?
            .map(|b| b.value()))
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: ethrex_common::H256,
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

    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self
                .read(
                    RECEIPTS_TABLE,
                    <(H256, u64) as Into<TupleRLP<BlockHash, Index>>>::into((hash, index)),
                )
                .await?
                .map(|b| b.value().to()))
        } else {
            Ok(None)
        }
    }

    fn get_account_code(
        &self,
        code_hash: ethrex_common::H256,
    ) -> Result<Option<bytes::Bytes>, StoreError> {
        Ok(self
            .read(
                ACCOUNT_CODES_TABLE,
                <H256 as Into<AccountCodeHashRLP>>::into(code_hash),
            )?
            .map(|b| b.value().to()))
    }

    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read(CANONICAL_BLOCK_HASHES_TABLE, block_number)
            .await
            .map(|o| o.map(|hash_rlp| hash_rlp.value().to()))
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

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read(CHAIN_DATA_TABLE, ChainDataIndex::EarliestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read(CHAIN_DATA_TABLE, ChainDataIndex::FinalizedBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read(CHAIN_DATA_TABLE, ChainDataIndex::SafeBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read(CHAIN_DATA_TABLE, ChainDataIndex::LatestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read(CHAIN_DATA_TABLE, ChainDataIndex::PendingBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(&rlp.value())
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(
        &self,
        hashed_address: ethrex_common::H256,
        storage_root: ethrex_common::H256,
    ) -> ethrex_trie::Trie {
        let db = Box::new(RedBMultiTableTrieDB::new(self.db.clone(), hashed_address.0));
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: ethrex_common::H256) -> ethrex_trie::Trie {
        let db = Box::new(RedBTrie::new(self.db.clone()));
        Trie::open(db, state_root)
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        Ok(self
            .read(PAYLOADS_TABLE, payload_id)
            .await?
            .map(|b| b.value().to()))
    }

    fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> std::result::Result<Vec<Receipt>, StoreError> {
        let mut encoded_receipts = vec![];
        let mut receipt_index = 0;
        let mut expected_key: TupleRLP<BlockHash, Index> = (*block_hash, 0).into();
        let table = self.tx.open_table(RECEIPTS_TABLE)?;
        // We're searching receipts for a block, the keys
        // for the receipt table are of the kind: rlp((BlockHash, Index)).
        // So we search for values in the db that match with this kind
        // of key, until we reach an Index that returns None
        // and we stop the search.
        // TODO(#1436): Make sure this if this is the proper way of
        // doing a search for each key, libmdbx has cursors
        // for this purpose, we should do the equal here,
        // if this approach is not correct.
        while let Some(access_guard) = table.get(&expected_key)? {
            encoded_receipts.push(access_guard.value());
            receipt_index += 1;
            expected_key = (*block_hash, receipt_index).into()
        }
        Ok(encoded_receipts
            .into_iter()
            .map(|receipt| receipt.to())
            .collect())
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::HeaderDownloadCheckpoint)
            .await?
            .map(|rlp| RLPDecode::decode(&rlp.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn get_state_trie_key_checkpoint(&self) -> Result<Option<[H256; 2]>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::StateTrieKeyCheckpoint)
            .await?
            .map(|rlp| {
                <Vec<H256>>::decode(&rlp.value())?
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)
            })
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        // Read values
        let start = <H256 as Into<AccountHashRLP>>::into(Default::default());
        let res: Vec<(H256, Vec<Nibbles>)> = self
            .tx
            .open_table(STORAGE_HEAL_PATHS_TABLE)?
            .extract_from_if(start..start + limit, |_, _| true)
            .collect();
        Ok(res)
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::StateHealPaths)
            .await?
            .map(|rlp| RLPDecode::decode(&rlp.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; crate::STATE_TRIE_SEGMENTS])>, StoreError> {
        let Some((root, checkpoints)) = self
            .read(SNAP_STATE_TABLE, SnapStateIndex::StateTrieRebuildCheckpoint)
            .await?
            .map(|ref rlp| <(H256, Vec<H256>)>::decode(&rlp.value()))
            .transpose()?
        else {
            return Ok(None);
        };
        Ok(Some((
            root,
            checkpoints
                .try_into()
                .map_err(|_| RLPDecodeError::InvalidLength)?,
        )))
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::StorageTrieRebuildPending)
            .await?
            .map(|p| RLPDecode::decode(&p.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn read_account_snapshot(
        &self,
        start: H256,
    ) -> Result<Vec<(H256, ethrex_common::types::AccountState)>, StoreError> {
        let table = self.tx.open_table(STATE_SNAPSHOT_TABLE)?;
        Ok(table
            .range(<H256 as Into<AccountHashRLP>>::into(start)..)?
            .take(MAX_SNAPSHOT_READS)
            .map_while(|elem| {
                elem.ok()
                    .map(|(key, value)| (key.value().to(), value.value().to()))
            })
            .collect())
    }

    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let table = self.tx.open_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
        Ok(table
            .get(<H256 as Into<AccountHashRLP>>::into(account_hash))?
            .map_while(|elem| {
                elem.ok().and_then(|entry| {
                    let (key, val) = entry.value();
                    if H256(key) < start {
                        None
                    } else {
                        Some((H256(key), U256::from_big_endian(&val)))
                    }
                })
            })
            .take(MAX_SNAPSHOT_READS)
            .collect())
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            .read(
                INVALID_ANCESTORS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block),
            )
            .await?
            .map(|b| b.value().to()))
    }
}

#[async_trait::async_trait]
impl StoreRwTx<'_> for RedBRwTx {
    async fn commit(self) -> Result<(), StoreError> {
        tokio::task::spawn_blocking(move || self.blocking_commit())
            .await
            .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }
    async fn rollback(self) -> Result<(), StoreError> {
        tokio::task::spawn_blocking(move || self.blocking_rollback())
            .await
            .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }
    fn blocking_commit(self) -> Result<(), StoreError> {
        Ok(self.tx.commit()?)
    }
    fn blocking_rollback(self) -> Result<(), StoreError> {
        Ok(self.tx.abort()?)
    }
    async fn add_block_header(
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

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let key_values = block_hashes
            .into_iter()
            .zip(block_headers)
            .map(|(hash, header)| {
                (
                    <H256 as Into<BlockHashRLP>>::into(hash),
                    <BlockHeader as Into<BlockHeaderRLP>>::into(header),
                )
            })
            .collect();
        self.write_batch(HEADERS_TABLE, key_values)
    }

    async fn add_block_body(
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

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let write_txn = &self.tx;
        let mut transaction_table = write_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;
        let mut headers_table = write_txn.open_table(HEADERS_TABLE)?;
        let mut block_bodies_table = write_txn.open_table(BLOCK_BODIES_TABLE)?;
        let mut block_numbers_table = write_txn.open_table(BLOCK_NUMBERS_TABLE)?;

        for block in blocks {
            let block_number = block.header.number;
            let block_hash = block.hash();

            for (index, transaction) in block.body.transactions.iter().enumerate() {
                transaction_table.insert(
                    <H256 as Into<TransactionHashRLP>>::into(transaction.compute_hash()),
                    <(u64, H256, u64) as Into<Rlp<(BlockNumber, BlockHash, Index)>>>::into((
                        block_number,
                        block_hash,
                        index as u64,
                    )),
                )?;
            }

            headers_table.insert(
                <H256 as Into<BlockHashRLP>>::into(block_hash),
                <BlockHeader as Into<BlockHeaderRLP>>::into(block.header.clone()),
            )?;
            block_bodies_table.insert(
                <H256 as Into<BlockHashRLP>>::into(block_hash),
                <BlockBody as Into<BlockBodyRLP>>::into(block.body.clone()),
            )?;
            block_numbers_table
                .insert(<H256 as Into<BlockHashRLP>>::into(block_hash), block_number)?;
        }
        Ok(())
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values = blocks
            .iter()
            .map(|e| {
                (
                    e.header.number,
                    <H256 as Into<BlockHashRLP>>::into(e.hash()),
                )
            })
            .collect();

        self.write_batch(CANONICAL_BLOCK_HASHES_TABLE, key_values)
            .await
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write(
            PENDING_BLOCKS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block.hash()),
            <Block as Into<BlockRLP>>::into(block),
        )
        .await
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            BLOCK_NUMBERS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
            block_number,
        )
        .await
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: ethrex_common::H256,
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
        .await
    }

    async fn add_receipt(
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
        .await
    }

    async fn add_receipts_for_blocks(
        &self,
        receipts: HashMap<BlockHash, Vec<Receipt>>,
    ) -> Result<(), StoreError> {
        let mut key_values = vec![];

        for (block_hash, receipts) in receipts.into_iter() {
            let mut kv = receipts
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

            key_values.append(&mut kv);
        }

        self.write_batch(RECEIPTS_TABLE, key_values).await
    }

    async fn add_account_code(
        &self,
        code_hash: ethrex_common::H256,
        code: bytes::Bytes,
    ) -> Result<(), StoreError> {
        self.write(
            ACCOUNT_CODES_TABLE,
            <H256 as Into<AccountCodeHashRLP>>::into(code_hash),
            <bytes::Bytes as Into<AccountCodeRLP>>::into(code),
        )
        .await
    }

    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
        .await
    }

    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn update_latest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write(
            CANONICAL_BLOCK_HASHES_TABLE,
            number,
            <H256 as Into<BlockHashRLP>>::into(hash),
        )
        .await
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.delete(CANONICAL_BLOCK_HASHES_TABLE, number)
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write(
            PAYLOADS_TABLE,
            payload_id,
            <PayloadBundle as Into<PayloadBundleRLP>>::into(PayloadBundle::from_block(block)),
        )
        .await
    }

    async fn add_receipts(
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
        self.write_batch(RECEIPTS_TABLE, key_values).await
    }

    async fn add_transaction_locations(
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

        self.write_to_multi_batch(TRANSACTION_LOCATIONS_TABLE, key_values)
            .await?;

        Ok(())
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.write(
            PAYLOADS_TABLE,
            payload_id,
            <PayloadBundle as Into<PayloadBundleRLP>>::into(payload),
        )
        .await
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::HeaderDownloadCheckpoint,
            block_hash.encode_to_vec(),
        )
        .await
    }

    async fn set_state_trie_key_checkpoint(&self, last_key: [H256; 2]) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StateTrieKeyCheckpoint,
            last_key.to_vec().encode_to_vec(),
        )
        .await
    }

    async fn set_storage_heal_paths(
        &self,
        paths: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        let key_values = paths
            .into_iter()
            .map(|(hash, paths)| {
                (
                    <H256 as Into<AccountHashRLP>>::into(hash),
                    <Vec<Nibbles> as Into<TriePathsRLP>>::into(paths),
                )
            })
            .collect();
        self.write_batch(STORAGE_HEAL_PATHS_TABLE, key_values).await
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StateHealPaths,
            paths.encode_to_vec(),
        )
        .await
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let write_txn = self.db.begin_write()?;
        // Delete the whole table as it will be re-crated when we next open it
        write_txn.delete_table(SNAP_STATE_TABLE)?;
        write_txn.commit()?;
        Ok(())
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<ethrex_common::types::AccountState>,
    ) -> Result<(), StoreError> {
        self.write_batch(
            STATE_SNAPSHOT_TABLE,
            account_hashes
                .into_iter()
                .map(<H256 as Into<AccountHashRLP>>::into)
                .zip(
                    account_states
                        .into_iter()
                        .map(<AccountState as Into<AccountStateRLP>>::into),
                )
                .collect::<Vec<_>>(),
        )
        .await
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let write_tx = self.db.begin_write()?;
        {
            let mut table = write_tx.open_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
            for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                table.insert(
                    <H256 as Into<AccountHashRLP>>::into(account_hash),
                    (key.0, value.to_big_endian()),
                )?;
            }
        }
        write_tx.commit()?;
        Ok(())
    }
    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        let write_tx = self.db.begin_write()?;
        {
            let mut table = write_tx.open_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
            for (account_hash, (storage_keys, storage_values)) in account_hashes
                .into_iter()
                .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
            {
                for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                    table.insert(
                        <H256 as Into<AccountHashRLP>>::into(account_hash),
                        (key.0, value.to_big_endian()),
                    )?;
                }
            }
        }
        write_tx.commit()?;
        Ok(())
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; crate::STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StateTrieRebuildCheckpoint,
            (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec(),
        )
        .await
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StorageTrieRebuildPending,
            pending.encode_to_vec(),
        )
        .await
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let write_tx = self.db.begin_write()?;
        write_tx.delete_table(STATE_SNAPSHOT_TABLE)?;
        write_tx.delete_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
        write_tx.commit()?;
        Ok(())
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        self.write(
            INVALID_ANCESTORS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(bad_block),
            <H256 as Into<BlockHashRLP>>::into(latest_valid),
        )
        .await
    }
}

impl redb::Value for ChainDataIndex {
    type SelfType<'a>
        = ChainDataIndex
    where
        Self: 'a;

    type AsBytes<'a>
        = [u8; 1]
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

impl redb::Value for SnapStateIndex {
    type SelfType<'a>
        = SnapStateIndex
    where
        Self: 'a;

    type AsBytes<'a>
        = [u8; 1]
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
        TypeName::new("SnapStateIndex")
    }
}

impl redb::Key for SnapStateIndex {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

pub fn init_db() -> Result<Database, StoreError> {
    let db = Database::create("ethrex.redb")?;

    let table_creation_txn = db.begin_write()?;
    table_creation_txn.open_table(STATE_TRIE_NODES_TABLE)?;
    table_creation_txn.open_table(BLOCK_NUMBERS_TABLE)?;
    table_creation_txn.open_table(CANONICAL_BLOCK_HASHES_TABLE)?;
    table_creation_txn.open_table(RECEIPTS_TABLE)?;
    table_creation_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;
    table_creation_txn.open_table(CHAIN_DATA_TABLE)?;
    table_creation_txn.open_table(BLOCK_BODIES_TABLE)?;
    table_creation_txn.open_table(PAYLOADS_TABLE)?;
    table_creation_txn.open_table(PENDING_BLOCKS_TABLE)?;
    table_creation_txn.open_table(INVALID_ANCESTORS_TABLE)?;
    table_creation_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;
    table_creation_txn.open_table(SNAP_STATE_TABLE)?;
    table_creation_txn.open_table(STATE_SNAPSHOT_TABLE)?;
    table_creation_txn.open_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
    table_creation_txn.commit()?;

    Ok(db)
}
