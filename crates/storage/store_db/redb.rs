use std::{borrow::Borrow, panic::RefUnwindSafe, sync::Arc};

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
    H256, U256,
    types::{
        Block, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index, Receipt,
        payload::PayloadBundle,
    },
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie};
use redb::{AccessGuard, Database, Key, MultimapTableDefinition, TableDefinition, TypeName, Value};

use crate::UpdateBatch;
use crate::trie_db::utils::node_hash_to_fixed_size;
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

impl RefUnwindSafe for RedBStore {}
impl RedBStore {
    pub fn new() -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db()?),
        })
    }

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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;
            write_txn.open_table(table)?.insert(key, value)?;
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;
            write_txn.open_multimap_table(table)?.insert(key, value)?;
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;
            {
                let mut table = write_txn.open_table(table)?;
                for (key, value) in key_values {
                    table.insert(key, value)?;
                }
            }
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;
            {
                let mut table = write_txn.open_multimap_table(table)?;
                for (key, value) in key_values {
                    table.insert(key, value)?;
                }
            }
            write_txn.commit()?;

            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let read_txn = db.begin_read().map_err(Box::new)?;
            let table = read_txn.open_table(table)?;
            let result = table.get(key)?;

            Ok(result)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a redb table
    fn read_sync<'k, 'a, K, V>(
        &self,
        table: TableDefinition<'a, K, V>,
        key: impl Borrow<K::SelfType<'k>>,
    ) -> Result<Option<AccessGuard<'static, V>>, StoreError>
    where
        K: Key + 'static,
        V: Value,
    {
        let read_txn = self.db.begin_read().map_err(Box::new)?;
        let table = read_txn.open_table(table)?;
        let result = table.get(key)?;

        Ok(result)
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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let read_txn = db.begin_read().map_err(Box::new)?;
            let table = read_txn.open_table(table)?;
            let mut result = Vec::new();
            for key in keys {
                if let Some(val) = table.get(key)? {
                    result.push(val);
                }
            }
            Ok(result)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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
        let write_txn = self.db.begin_write().map_err(Box::new)?;
        write_txn.open_table(table)?.remove(key)?;
        write_txn.commit()?;

        Ok(())
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync(CANONICAL_BLOCK_HASHES_TABLE, number)?
            .map(|a| a.value().to())
            .transpose()
            .map_err(StoreError::from)
    }
}

#[async_trait::async_trait]
impl StoreEngine for RedBStore {
    async fn apply_storage_trie_changes(
        &self,
        _changeset: HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
    ) -> Result<(), StoreError> {
        todo!();
    }

    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;
            {
                // store account updates
                let mut state_trie_store = write_txn.open_table(STATE_TRIE_NODES_TABLE)?;
                for (node_hash, node_data) in update_batch.account_updates {
                    state_trie_store.insert(node_hash.as_ref(), &*node_data)?;
                }

                // store code updates
                let mut code_store = write_txn.open_table(ACCOUNT_CODES_TABLE)?;
                for (hashed_address, code) in update_batch.code_updates {
                    let account_code_hash = <H256 as Into<AccountCodeHashRLP>>::into(hashed_address);
                    let account_code = <bytes::Bytes as Into<AccountCodeRLP>>::into(code);
                    code_store.insert(account_code_hash, account_code)?;
                }

                let mut addr_store = write_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;
                for (hashed_address, nodes) in update_batch.storage_updates {
                    for (node_hash, node_data) in nodes {
                        addr_store.insert(
                            (hashed_address.0, node_hash_to_fixed_size(node_hash)),
                            &*node_data,
                        )?;
                    }
                }

                let mut transaction_table = write_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;
                let mut bodies = write_txn.open_table(BLOCK_BODIES_TABLE)?;
                let mut headers = write_txn.open_table(HEADERS_TABLE)?;
                let mut block_numbers = write_txn.open_table(BLOCK_NUMBERS_TABLE)?;

                for block in update_batch.blocks {
                    // store block
                    let number = block.header.number;
                    let hash = <H256 as Into<BlockHashRLP>>::into(block.hash());

                    for (index, transaction) in block.body.transactions.iter().enumerate() {
                        transaction_table.insert(
                            <H256 as Into<TransactionHashRLP>>::into(transaction.compute_hash()),
                            <(u64, BlockHash, u64) as Into<Rlp<(BlockNumber, BlockHash, Index)>>>::into(
                                (number, block.hash(), index as u64),
                            ),
                        )?;
                    }
                    bodies.insert(
                        hash.clone(),
                        <BlockBody as Into<BlockBodyRLP>>::into(block.body),
                    )?;
                    headers.insert(
                        hash.clone(),
                        <BlockHeader as Into<BlockHeaderRLP>>::into(block.header),
                    )?;
                    block_numbers.insert(hash, number)?;
                }

                let mut receipts_table = write_txn.open_table(RECEIPTS_TABLE)?;
                for (block_hash, receipts) in update_batch.receipts {
                    for (index, receipt) in receipts.into_iter().enumerate() {
                        receipts_table.insert(
                            <(BlockHash, u64) as Into<TupleRLP<BlockHash, Index>>>::into((
                                block_hash,
                                index as u64,
                            )),
                            <Receipt as Into<ReceiptRLP>>::into(receipt),
                        )?;
                    }
                }
            }

            write_txn.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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
        .await
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
        self.write_batch(HEADERS_TABLE, key_values).await
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.read_sync(HEADERS_TABLE, <H256 as Into<BlockHashRLP>>::into(hash))?
                .map(|b| b.value().to())
                .transpose()
                .map_err(StoreError::from)
        } else {
            Ok(None)
        }
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
        .await
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let write_txn = db.begin_write().map_err(Box::new)?;

            {
                // Begin block so that tables are opened once and dropped at the end.
                // This prevents ownership errors when to committing changes at the end.
                {
                    let mut transaction_table =
                        write_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;
                    let mut headers_table = write_txn.open_table(HEADERS_TABLE)?;
                    let mut block_bodies_table = write_txn.open_table(BLOCK_BODIES_TABLE)?;
                    let mut block_numbers_table = write_txn.open_table(BLOCK_NUMBERS_TABLE)?;

                    for block in blocks {
                        let block_number = block.header.number;
                        let block_hash = block.hash();

                        for (index, transaction) in block.body.transactions.iter().enumerate() {
                            transaction_table.insert(
                                <H256 as Into<TransactionHashRLP>>::into(transaction.compute_hash()),
                                <(u64, H256, u64) as Into<Rlp<(BlockNumber, BlockHash, Index)>>>::into(
                                    (block_number, block_hash, index as u64),
                                ),
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
                }

                write_txn.commit()?;

                Ok(())
            }
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
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

    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let Some(hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(());
        };
        let hash = <H256 as Into<BlockHashRLP>>::into(hash);
        let write_txn = self.db.begin_write().map_err(Box::new)?;

        write_txn
            .open_table(CANONICAL_BLOCK_HASHES_TABLE)?
            .remove(block_number)?;
        write_txn.open_table(BLOCK_BODIES_TABLE)?.remove(&hash)?;
        write_txn.open_table(HEADERS_TABLE)?.remove(&hash)?;
        write_txn.open_table(BLOCK_NUMBERS_TABLE)?.remove(&hash)?;

        write_txn.commit()?;
        Ok(())
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
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.value().to()?)
        }
        Ok(block_bodies)
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
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.value().to()?)
        }
        Ok(block_bodies)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.read(
            BLOCK_BODIES_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
        )
        .await?
        .map(|b| b.value().to())
        .transpose()
        .map_err(StoreError::from)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.read_sync(
            HEADERS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
        )?
        .map(|b| b.value().to())
        .transpose()
        .map_err(StoreError::from)
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write(
            PENDING_BLOCKS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block.hash()),
            <Block as Into<BlockRLP>>::into(block),
        )
        .await
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        self.read(
            PENDING_BLOCKS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block_hash),
        )
        .await?
        .map(|b| b.value().to())
        .transpose()
        .map_err(StoreError::from)
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

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        Ok(self
            .read_sync(
                BLOCK_NUMBERS_TABLE,
                <H256 as Into<BlockHashRLP>>::into(block_hash),
            )?
            .map(|b| b.value()))
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

    async fn get_transaction_location(
        &self,
        transaction_hash: ethrex_common::H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let read_txn = self.db.begin_read().map_err(Box::new)?;
        let table = read_txn.open_multimap_table(TRANSACTION_LOCATIONS_TABLE)?;

        let mut table_vec = Vec::new();
        while let Some(Ok(res)) = table
            .get(<H256 as Into<TransactionHashRLP>>::into(transaction_hash))?
            .next()
        {
            table_vec.push(res.value().to()?)
        }

        Ok(table_vec.into_iter().find(|(number, hash, _index)| {
            self.get_block_hash_by_block_number(*number)
                .is_ok_and(|o| o == Some(*hash))
        }))
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

    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.read(
                RECEIPTS_TABLE,
                <(H256, u64) as Into<TupleRLP<BlockHash, Index>>>::into((hash, index)),
            )
            .await?
            .map(|b| b.value().to())
            .transpose()
            .map_err(StoreError::from)
        } else {
            Ok(None)
        }
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

    fn get_account_code(
        &self,
        code_hash: ethrex_common::H256,
    ) -> Result<Option<bytes::Bytes>, StoreError> {
        self.read_sync(
            ACCOUNT_CODES_TABLE,
            <H256 as Into<AccountCodeHashRLP>>::into(code_hash),
        )?
        .map(|b| b.value().to())
        .transpose()
        .map_err(StoreError::from)
    }

    async fn get_canonical_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read(CANONICAL_BLOCK_HASHES_TABLE, block_number)
            .await
            .map(|o| o.map(|hash_rlp| hash_rlp.value().to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_canonical_block_hash_sync(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            .read_sync(CANONICAL_BLOCK_HASHES_TABLE, block_number)
            .map(|o| o.map(|hash_rlp| hash_rlp.value().to()))?
            .transpose()?)
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

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read_sync(CHAIN_DATA_TABLE, ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes.value()).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
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

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write(
            CHAIN_DATA_TABLE,
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
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
    ) -> Result<ethrex_trie::Trie, StoreError> {
        let db = Box::new(RedBMultiTableTrieDB::new(self.db.clone(), hashed_address.0));
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(
        &self,
        state_root: ethrex_common::H256,
    ) -> Result<ethrex_trie::Trie, StoreError> {
        let db = Box::new(RedBTrie::new(self.db.clone()));
        Ok(Trie::open(db, state_root))
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

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        self.read(PAYLOADS_TABLE, payload_id)
            .await?
            .map(|b| b.value().to())
            .transpose()
            .map_err(StoreError::from)
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

    fn get_receipts_for_block(
        &self,
        block_hash: &BlockHash,
    ) -> std::result::Result<Vec<Receipt>, StoreError> {
        let mut encoded_receipts = vec![];
        let mut receipt_index = 0;
        let read_tx = self.db.begin_read().map_err(Box::new)?;
        let mut expected_key: TupleRLP<BlockHash, Index> = (*block_hash, 0).into();
        let table = read_tx.open_table(RECEIPTS_TABLE)?;
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
        let mut decoded_receipts = Vec::new();
        for encoded_receipt in encoded_receipts.into_iter() {
            decoded_receipts.push(encoded_receipt.to()?)
        }
        Ok(decoded_receipts)
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

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::HeaderDownloadCheckpoint)
            .await?
            .map(|rlp| RLPDecode::decode(&rlp.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_state_trie_key_checkpoint(&self, last_key: [H256; 2]) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StateTrieKeyCheckpoint,
            last_key.to_vec().encode_to_vec(),
        )
        .await
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

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        // Read values
        let txn = self.db.begin_read().map_err(Box::new)?;
        let table = txn.open_table(STORAGE_HEAL_PATHS_TABLE)?;
        let mut res: Vec<(H256, Vec<Nibbles>)> = Vec::new();
        while let Some(Ok((hash, paths))) = table
            .range(<H256 as Into<AccountHashRLP>>::into(Default::default())..)?
            .next()
        {
            res.push((hash.value().to()?, paths.value().to()?));
        }

        res = res.into_iter().take(limit).collect();
        txn.close().map_err(Box::new)?;
        // Delete read values
        let txn = self.db.begin_write().map_err(Box::new)?;
        {
            let mut table = txn.open_table(STORAGE_HEAL_PATHS_TABLE)?;
            for (hash, _) in res.iter() {
                table.remove(<H256 as Into<AccountHashRLP>>::into(*hash))?;
            }
        }
        txn.commit()?;
        Ok(res)
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.write(
            SNAP_STATE_TABLE,
            SnapStateIndex::StateHealPaths,
            paths.encode_to_vec(),
        )
        .await
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::StateHealPaths)
            .await?
            .map(|rlp| RLPDecode::decode(&rlp.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let write_txn = self.db.begin_write().map_err(Box::new)?;
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
        let write_tx = self.db.begin_write().map_err(Box::new)?;
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
        let write_tx = self.db.begin_write().map_err(Box::new)?;
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

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.read(SNAP_STATE_TABLE, SnapStateIndex::StorageTrieRebuildPending)
            .await?
            .map(|p| RLPDecode::decode(&p.value()))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let write_tx = self.db.begin_write().map_err(Box::new)?;
        write_tx.delete_table(STATE_SNAPSHOT_TABLE)?;
        write_tx.delete_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
        write_tx.commit()?;
        Ok(())
    }

    async fn read_account_snapshot(
        &self,
        start: H256,
    ) -> Result<Vec<(H256, ethrex_common::types::AccountState)>, StoreError> {
        let read_tx = self.db.begin_read().map_err(Box::new)?;
        let table = read_tx.open_table(STATE_SNAPSHOT_TABLE)?;
        let mut table_vec = Vec::new();
        while let Some(Ok((key, value))) = table
            .range(<H256 as Into<AccountHashRLP>>::into(start)..)?
            .take(MAX_SNAPSHOT_READS)
            .next()
        {
            table_vec.push((key.value().to()?, value.value().to()?));
        }

        Ok(table_vec)
    }

    async fn read_storage_snapshot(
        &self,
        start: H256,
        account_hash: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let read_tx = self.db.begin_read().map_err(Box::new)?;
        let table = read_tx.open_multimap_table(STORAGE_SNAPSHOT_TABLE)?;
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
        self.read(
            INVALID_ANCESTORS_TABLE,
            <H256 as Into<BlockHashRLP>>::into(block),
        )
        .await?
        .map(|b| b.value().to())
        .transpose()
        .map_err(StoreError::from)
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

    let table_creation_txn = db.begin_write().map_err(Box::new)?;
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
