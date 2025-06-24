use crate::UpdateBatch;
use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
    BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, Rlp, TransactionHashRLP,
    TriePathsRLP, TupleRLP,
};
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::trie_db::libmdbx::LibmdbxTrieDB;
use crate::trie_db::libmdbx_dupsort::LibmdbxDupsortTrieDB;
use crate::trie_db::utils::node_hash_to_fixed_size;
use crate::utils::{ChainDataIndex, SnapStateIndex};
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig, Index,
    Receipt, Transaction, payload::PayloadBundle,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, NodeHash, Trie};
use libmdbx::orm::{Decodable, DupSort, Encodable, Table};
use libmdbx::{DatabaseOptions, Mode, PageSize, ReadWriteOptions, TransactionKind};
use libmdbx::{
    dupsort,
    orm::{Database, table},
    table_info,
};
use serde_json;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::info;

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db(Some(path)).map_err(StoreError::LibmdbxError)?),
        })
    }

    // Helper method to write into a libmdbx table
    async fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.upsert::<T>(key, value)
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to write into a libmdbx table in batch
    async fn write_batch<T: Table>(
        &self,
        key_values: Vec<(T::Key, T::Value)>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            let mut cursor = txn.cursor::<T>().map_err(StoreError::LibmdbxError)?;
            for (key, value) in key_values {
                cursor
                    .upsert(key, value)
                    .map_err(StoreError::LibmdbxError)?;
            }
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_read().map_err(StoreError::LibmdbxError)?;
            txn.get::<T>(key).map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read_bulk<T: Table>(&self, keys: Vec<T::Key>) -> Result<Vec<T::Value>, StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut res = Vec::new();
            let txn = db.begin_read().map_err(StoreError::LibmdbxError)?;
            for key in keys {
                let val = txn.get::<T>(key).map_err(StoreError::LibmdbxError)?;
                match val {
                    Some(val) => res.push(val),
                    None => Err(StoreError::ReadError)?,
                }
            }
            Ok(res)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    fn read_sync<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        txn.get::<T>(key).map_err(StoreError::LibmdbxError)
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync::<CanonicalBlockHashes>(number)?
            .map(|block_hash| block_hash.to())
            .transpose()
            .map_err(StoreError::from)
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            // store account updates
            for (node_hash, node_data) in update_batch.account_updates {
                tx.upsert::<StateTrieNodes>(node_hash, node_data)
                    .map_err(StoreError::LibmdbxError)?;
            }

            // store code updates
            for (hashed_address, code) in update_batch.code_updates {
                tx.upsert::<AccountCodes>(hashed_address.into(), code.into())
                    .map_err(StoreError::LibmdbxError)?;
            }

            for (hashed_address, nodes) in update_batch.storage_updates {
                for (node_hash, node_data) in nodes {
                    let key_1: [u8; 32] = hashed_address.into();
                    let key_2 = node_hash_to_fixed_size(node_hash);

                    tx.upsert::<StorageTriesNodes>((key_1, key_2), node_data)
                        .map_err(StoreError::LibmdbxError)?;
                }
            }
            for block in update_batch.blocks {
                // store block
                let number = block.header.number;
                let hash = block.hash();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.upsert::<TransactionLocations>(
                        transaction.compute_hash().into(),
                        (number, hash, index as u64).into(),
                    )
                    .map_err(StoreError::LibmdbxError)?;
                }

                tx.upsert::<Bodies>(
                    hash.into(),
                    BlockBodyRLP::from_bytes(block.body.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<Headers>(
                    hash.into(),
                    BlockHeaderRLP::from_bytes(block.header.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<BlockNumbers>(hash.into(), number)
                    .map_err(StoreError::LibmdbxError)?;
            }
            for (block_hash, receipts) in update_batch.receipts {
                // store receipts
                let mut key_values: Vec<(Rlp<(H256, u64)>, IndexedChunk<Receipt>)> = vec![];
                for mut entries in
                    receipts
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, receipt)| {
                            let key = (block_hash, index as u64).into();
                            let receipt_rlp = receipt.encode_to_vec();
                            IndexedChunk::from::<Receipts>(key, &receipt_rlp)
                        })
                {
                    key_values.append(&mut entries);
                }
                let mut cursor = tx.cursor::<Receipts>().map_err(StoreError::LibmdbxError)?;
                for (key, value) in key_values {
                    cursor
                        .upsert(key, value)
                        .map_err(StoreError::LibmdbxError)?;
                }
            }

            tx.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write::<Headers>(block_hash.into(), block_header.into())
            .await
    }

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let hashes_and_headers = block_hashes
            .into_iter()
            .zip(block_headers)
            .map(|(hash, header)| (hash.into(), header.into()))
            .collect();
        self.write_batch::<Headers>(hashes_and_headers).await
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };

        self.read_sync::<Headers>(block_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write::<Bodies>(block_hash.into(), block_body.into())
            .await
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            for block in blocks {
                let number = block.header.number;
                let hash = block.hash();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.upsert::<TransactionLocations>(
                        transaction.compute_hash().into(),
                        (number, hash, index as u64).into(),
                    )
                    .map_err(StoreError::LibmdbxError)?;
                }

                tx.upsert::<Bodies>(
                    hash.into(),
                    BlockBodyRLP::from_bytes(block.body.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<Headers>(
                    hash.into(),
                    BlockHeaderRLP::from_bytes(block.header.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<BlockNumbers>(hash.into(), number)
                    .map_err(StoreError::LibmdbxError)?;
            }

            tx.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values = blocks
            .iter()
            .map(|e| (e.header.number, e.hash().into()))
            .collect();

        self.write_batch::<CanonicalBlockHashes>(key_values).await
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
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;

        txn.delete::<CanonicalBlockHashes>(block_number, None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<Bodies>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<Headers>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<BlockNumbers>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;

        txn.commit().map_err(StoreError::LibmdbxError)
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers = (from..=to).collect();
        let hashes = self.read_bulk::<CanonicalBlockHashes>(numbers).await?;
        let blocks = self.read_bulk::<Bodies>(hashes).await?;
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.to()?)
        }
        Ok(block_bodies)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let hashes = hashes.into_iter().map(|h| h.into()).collect();
        let blocks = self.read_bulk::<Bodies>(hashes).await?;
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.to()?)
        }
        Ok(block_bodies)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.read::<Bodies>(block_hash.into())
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.read_sync::<Headers>(block_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<BlockNumbers>(block_hash.into(), block_number)
            .await
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read::<BlockNumbers>(block_hash.into()).await
    }

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read_sync::<BlockNumbers>(block_hash.into())
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.write::<AccountCodes>(code_hash.into(), code.into())
            .await
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.read_sync::<AccountCodes>(code_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key: Rlp<(BlockHash, Index)> = (block_hash, index).into();
        let Some(entries) = IndexedChunk::from::<Receipts>(key, &receipt.encode_to_vec()) else {
            return Err(StoreError::Custom("Invalid size".to_string()));
        };
        self.write_batch::<Receipts>(entries).await
    }

    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
            let mut cursor = txn.cursor::<Receipts>().map_err(StoreError::LibmdbxError)?;
            let key = (hash, index).into();
            IndexedChunk::read_from_db(&mut cursor, key)
        } else {
            Ok(None)
        }
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.write::<TransactionLocations>(
            transaction_hash.into(),
            (block_number, block_hash, index).into(),
        )
        .await
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<TransactionLocations>()
            .map_err(StoreError::LibmdbxError)?;

        let mut transaction_hashes = Vec::new();
        let mut cursor_it = cursor.walk_key(transaction_hash.into(), None);
        while let Some(Ok(tx)) = cursor_it.next() {
            transaction_hashes.push(tx.to().map_err(StoreError::from)?);
        }

        Ok(transaction_hashes
            .into_iter()
            .find(|(number, hash, _index)| {
                self.get_block_hash_by_block_number(*number)
                    .is_ok_and(|o| o == Some(*hash))
            }))
    }

    /// Stores the chain config serialized as json
    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
        .await
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read_sync::<ChainData>(ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
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
        self.write::<ChainData>(
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::EarliestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::FinalizedBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::SafeBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_latest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::LatestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::PendingBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = Box::new(LibmdbxDupsortTrieDB::<StorageTriesNodes, [u8; 32]>::new(
            self.db.clone(),
            hashed_address.0,
        ));
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(LibmdbxTrieDB::<StateTrieNodes>::new(self.db.clone()));
        Ok(Trie::open(db, state_root))
    }

    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<CanonicalBlockHashes>(number, hash.into())
            .await
    }

    async fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<CanonicalBlockHashes>(number)
            .await
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_canonical_block_hash_sync(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync::<CanonicalBlockHashes>(number)
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, PayloadBundle::from_block(block).into())
            .await
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        Ok(self
            .read::<Payloads>(payload_id)
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)?)
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, payload.into()).await
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash).await? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index).await
    }

    async fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    async fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.begin_readwrite()
                .map_err(StoreError::LibmdbxError)?
                .delete::<CanonicalBlockHashes>(number, None)
                .map(|_| ())
                .map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write::<PendingBlocks>(block.hash().into(), block.into())
            .await
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        self.read::<PendingBlocks>(block_hash.into())
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        #[allow(clippy::type_complexity)]
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (tx_hash.into(), (block_number, block_hash, index).into())
            })
            .collect();

        self.write_batch::<TransactionLocations>(key_values).await
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut key_values = vec![];

        for (index, receipt) in receipts.clone().into_iter().enumerate() {
            let key = (block_hash, index as u64).into();
            let receipt_rlp = receipt.encode_to_vec();
            let Some(mut entries) = IndexedChunk::from::<Receipts>(key, &receipt_rlp) else {
                continue;
            };

            key_values.append(&mut entries);
        }

        self.write_batch::<Receipts>(key_values).await
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = vec![];
        let mut receipt_index = 0;
        let mut key = (*block_hash, 0).into();
        let txn = self.db.begin_read().map_err(|_| StoreError::ReadError)?;
        let mut cursor = txn
            .cursor::<Receipts>()
            .map_err(|_| StoreError::CursorError("Receipts".to_owned()))?;

        // We're searching receipts for a block, the keys
        // for the receipt table are of the kind: rlp((BlockHash, Index)).
        // So we search for values in the db that match with this kind
        // of key, until we reach an Index that returns None
        // and we stop the search.
        while let Some(receipt) = IndexedChunk::read_from_db(&mut cursor, key)? {
            receipts.push(receipt);
            receipt_index += 1;
            key = (*block_hash, receipt_index).into();
        }

        Ok(receipts)
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint,
            block_hash.encode_to_vec(),
        )
        .await
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint)
            .await?
            .map(|ref h| BlockHash::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieKeyCheckpoint,
            last_keys.to_vec().encode_to_vec(),
        )
        .await
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint)
            .await?
            .map(|ref c| {
                <Vec<H256>>::decode(c)?
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
        self.write_batch::<StorageHealPaths>(
            paths
                .into_iter()
                .map(|(hash, paths)| (hash.into(), paths.into()))
                .collect(),
        )
        .await
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StorageHealPaths>()
            .map_err(StoreError::LibmdbxError)?;

        let mut res = Vec::new();
        let mut cursor_it = cursor.walk(None);
        while let Some(Ok((hash, paths))) = cursor_it.next() {
            res.push((hash.to()?, paths.to()?));
        }

        res = res.into_iter().take(limit).collect::<Vec<_>>();

        // Delete fetched entries from the table
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        for (hash, _) in res.iter() {
            txn.delete::<StorageHealPaths>((*hash).into(), None)
                .map_err(StoreError::LibmdbxError)?;
        }
        txn.commit().map_err(StoreError::LibmdbxError)?;
        Ok(res)
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.write::<SnapState>(SnapStateIndex::StateHealPaths, paths.encode_to_vec())
            .await
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateHealPaths)
            .await?
            .map(|ref h| <Vec<Nibbles>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<SnapState>()
                .map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StorageHealPaths>()
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        self.write_batch::<StateSnapShot>(
            account_hashes
                .into_iter()
                .map(|h| h.into())
                .zip(account_states.into_iter().map(|a| a.into()))
                .collect(),
        )
        .await
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                txn.upsert::<StorageSnapShot>(account_hash.into(), (key.into(), value.into()))
                    .map_err(StoreError::LibmdbxError)?;
            }

            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            for (account_hash, (storage_keys, storage_values)) in account_hashes
                .into_iter()
                .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
            {
                for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                    txn.upsert::<StorageSnapShot>(account_hash.into(), (key.into(), value.into()))
                        .map_err(StoreError::LibmdbxError)?;
                }
            }
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieRebuildCheckpoint,
            (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec(),
        )
        .await
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let Some((root, checkpoints)) = self
            .read::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint)
            .await?
            .map(|ref c| <(H256, Vec<H256>)>::decode(c))
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
        self.write::<SnapState>(
            SnapStateIndex::StorageTrieRebuildPending,
            pending.encode_to_vec(),
        )
        .await
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StorageTrieRebuildPending)
            .await?
            .map(|ref h| <Vec<(H256, H256)>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StateSnapShot>()
                .map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StorageSnapShot>()
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StateSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        let iter = cursor
            .walk(Some(start.into()))
            .map_while(|res| {
                res.ok().map(|(hash, acc)| match (hash.to(), acc.to()) {
                    (Ok(hash), Ok(acc)) => Some((hash, acc)),
                    _ => None,
                })
            })
            .flatten()
            .take(MAX_SNAPSHOT_READS);
        Ok(iter.collect::<Vec<_>>())
    }

    async fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StorageSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        let iter = cursor
            .walk_key(account_hash.into(), Some(start.into()))
            .map_while(|res| {
                res.ok()
                    .map(|(k, v)| (H256(k.0), U256::from_big_endian(&v.0)))
            })
            .take(MAX_SNAPSHOT_READS);
        Ok(iter.collect::<Vec<_>>())
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<InvalidAncestors>(block.into())
            .await
            .map(|o| o.map(|a| a.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<InvalidAncestors>(bad_block.into(), latest_valid.into())
            .await
    }

    async fn apply_storage_trie_changes(
        &self,
        changeset: HashMap<H256, Vec<(NodeHash, Vec<u8>)>>,
    ) -> Result<(), StoreError> {
        let s = Instant::now();
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        let txn_create = s.elapsed().as_millis();
        let mut total_upserts = 0;
        for (acc_hash, nodes) in changeset {
            total_upserts += nodes.len();
            for (hash, node) in nodes {
                txn.upsert::<StorageTriesNodes>((acc_hash.0, node_hash_to_fixed_size(hash)), node)
                    .map_err(StoreError::LibmdbxError)?;
            }
        }
        let data_upsert = s.elapsed().as_millis() - txn_create;
        txn.commit().map_err(StoreError::LibmdbxError)?;
        let total = s.elapsed().as_millis();
        let txn_commit = total - txn_create - data_upsert;
        info!(
            "[apply_storage_trie_changes] put_batch: {total}ms: txn_create: {txn_create}ms; upserts: {total_upserts} in {data_upsert}ms; txn_commit: {txn_commit}ms"
        );
        Ok(())
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libmdbx Store").finish()
    }
}

// Define tables

/// For `dupsort` tables, multiple values can be stored under the same key.
/// To maintain an explicit order, each value is assigned an `index`.
/// This is useful when storing large byte sequences that exceed the maximum size limit,
/// requiring them to be split into smaller chunks for storage.
pub struct IndexedChunk<T: RLPEncode + RLPDecode> {
    index: u8,
    value: Rlp<T>,
}

pub trait ChunkTrait<T: RLPEncode + RLPDecode> {
    #[allow(unused)]
    fn index(&self) -> u8;
    fn value_bytes(&self) -> &Vec<u8>;
}

impl<T: RLPEncode + RLPDecode> ChunkTrait<T> for IndexedChunk<T> {
    fn index(&self) -> u8 {
        self.index
    }

    fn value_bytes(&self) -> &Vec<u8> {
        self.value.bytes()
    }
}

impl<T: Send + Sync + RLPEncode + RLPDecode> Decodable for IndexedChunk<T> {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        let index = b[0];
        let value = Rlp::from_bytes(b[1..].to_vec());
        Ok(Self { index, value })
    }
}

impl<T: Send + Sync + RLPEncode + RLPDecode> Encodable for IndexedChunk<T> {
    type Encoded = Vec<u8>;

    fn encode(self) -> Self::Encoded {
        // by appending the index at the begging, we enforce the btree ordering from lowest to highest
        let mut buf = vec![self.index];
        buf.extend_from_slice(self.value.bytes());
        buf
    }
}

impl<T: RLPEncode + RLPDecode> IndexedChunk<T> {
    /// Splits a value into a indexed chunks if it exceeds the maximum storage size.
    /// Each chunk is assigned an index to ensure correct ordering when retrieved.
    ///
    /// Warning: The current implementation supports a maximum of 256 chunks per value
    /// because the index is stored as a u8.
    ///
    /// If the data exceeds this limit, `None` is returned to indicate that it cannot be stored.
    pub fn from<Tab: Table>(key: Tab::Key, bytes: &[u8]) -> Option<Vec<(Tab::Key, Self)>>
    where
        Tab::Key: Clone,
    {
        let chunks: Vec<Vec<u8>> = bytes
            // -1 to account for the index byte
            .chunks(DB_MAX_VALUE_SIZE - 1)
            .map(|i| i.to_vec())
            .collect();

        if chunks.len() > 256 {
            return None;
        }

        let chunks = chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| {
                (
                    key.clone(),
                    IndexedChunk {
                        index: index as u8,
                        value: Rlp::from_bytes(chunk),
                    },
                )
            })
            .collect();

        Some(chunks)
    }

    /// Reads multiple stored chunks and reconstructs the original full value.
    /// The chunks are appended in order based on their assigned index.
    pub fn read_from_db<Tab: Table + DupSort, K: TransactionKind>(
        cursor: &mut libmdbx::orm::Cursor<'_, K, Tab>,
        key: Tab::Key,
    ) -> Result<Option<T>, StoreError>
    where
        Tab::Key: Decodable,
        Tab::Value: ChunkTrait<T>,
    {
        let mut value = vec![];

        if let Some((_, chunk)) = cursor.seek_exact(key).map_err(StoreError::LibmdbxError)? {
            value.extend_from_slice(chunk.value_bytes());
        } else {
            return Ok(None);
        }

        // Fetch remaining parts
        while let Some((_, chunk)) = cursor.next_value().map_err(StoreError::LibmdbxError)? {
            value.extend_from_slice(chunk.value_bytes());
        }

        let decoded = T::decode(&value).map_err(StoreError::RLPDecode)?;
        Ok(Some(decoded))
    }
}

table!(
    /// The canonical block hash for each block number. It represents the canonical chain.
    ( CanonicalBlockHashes ) BlockNumber => BlockHashRLP
);

table!(
    /// Block hash to number table.
    ( BlockNumbers ) BlockHashRLP => BlockNumber
);

table!(
    /// Block headers table.
    ( Headers ) BlockHashRLP => BlockHeaderRLP
);
table!(
    /// Block bodies table.
    ( Bodies ) BlockHashRLP => BlockBodyRLP
);
table!(
    /// Account codes table.
    ( AccountCodes ) AccountCodeHashRLP => AccountCodeRLP
);

dupsort!(
    /// Receipts table.
    ( Receipts ) TupleRLP<BlockHash, Index>[Index] => IndexedChunk<Receipt>
);

dupsort!(
    /// Table containing all storage trie's nodes
    /// Each node is stored by hashed account address and node hash in order to keep different storage trie's nodes separate
    ( StorageTriesNodes ) ([u8;32], [u8;33])[[u8;32]] => Vec<u8>
);

dupsort!(
    /// Transaction locations table.
    ( TransactionLocations ) TransactionHashRLP => Rlp<(BlockNumber, BlockHash, Index)>
);

table!(
    /// Stores chain data, each value is unique and stored as its rlp encoding
    /// See [ChainDataIndex] for available chain values
    ( ChainData ) ChainDataIndex => Vec<u8>
);

table!(
    /// Stores snap state, each value is unique and stored as its rlp encoding
    /// See [SnapStateIndex] for available values
    ( SnapState ) SnapStateIndex => Vec<u8>
);

// Trie storages

table!(
    /// state trie nodes
    ( StateTrieNodes ) NodeHash => Vec<u8>
);

// Local Blocks

table!(
    /// payload id to payload table
    ( Payloads ) u64 => PayloadBundleRLP
);

table!(
    /// Stores blocks that are pending validation.
    ( PendingBlocks ) BlockHashRLP => BlockRLP
);

table!(
    /// State Snapshot used by an ongoing sync process
    ( StateSnapShot ) AccountHashRLP => AccountStateRLP
);

dupsort!(
    /// Storage Snapshot used by an ongoing sync process
    ( StorageSnapShot ) AccountHashRLP => (AccountStorageKeyBytes, AccountStorageValueBytes)[AccountStorageKeyBytes]
);

table!(
    /// Storage trie paths in need of healing stored by hashed address
    ( StorageHealPaths ) AccountHashRLP => TriePathsRLP
);

table!(
    /// Stores invalid ancestors
    ( InvalidAncestors ) BlockHashRLP => BlockHashRLP
);

// Storage values are stored as bytes instead of using their rlp encoding
// As they are stored in a dupsort table, they need to have a fixed size, and encoding them doesn't preserve their size
pub struct AccountStorageKeyBytes(pub [u8; 32]);
pub struct AccountStorageValueBytes(pub [u8; 32]);

impl Encodable for AccountStorageKeyBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageKeyBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageKeyBytes(b.try_into()?))
    }
}

impl Encodable for AccountStorageValueBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageValueBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageValueBytes(b.try_into()?))
    }
}

impl From<H256> for AccountStorageKeyBytes {
    fn from(value: H256) -> Self {
        AccountStorageKeyBytes(value.0)
    }
}

impl From<U256> for AccountStorageValueBytes {
    fn from(value: U256) -> Self {
        AccountStorageValueBytes(value.to_big_endian())
    }
}

impl From<AccountStorageKeyBytes> for H256 {
    fn from(value: AccountStorageKeyBytes) -> Self {
        H256(value.0)
    }
}

impl From<AccountStorageValueBytes> for U256 {
    fn from(value: AccountStorageValueBytes) -> Self {
        U256::from_big_endian(&value.0)
    }
}

impl Encodable for ChainDataIndex {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        (self as u32).encode()
    }
}

impl Encodable for SnapStateIndex {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        (self as u32).encode()
    }
}

/// default page size recommended by libmdbx
///
/// - See here: https://github.com/erthink/libmdbx/tree/master?tab=readme-ov-file#limitations
/// - and here: https://libmdbx.dqdkfa.ru/structmdbx_1_1env_1_1geometry.html#a45048bf2de9120d01dae2151c060d459
const DB_PAGE_SIZE: usize = 4096;
/// For a default page size of 4096, the max value size is roughly 1/2 page size.
const DB_MAX_VALUE_SIZE: usize = 2022;
// Maximum DB size, set to 2 TB
const MAX_MAP_SIZE: isize = 1024_isize.pow(4) * 2; // 2 TB

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> anyhow::Result<Database> {
    let tables = [
        table_info!(BlockNumbers),
        table_info!(Headers),
        table_info!(Bodies),
        table_info!(AccountCodes),
        table_info!(Receipts),
        table_info!(TransactionLocations),
        table_info!(ChainData),
        table_info!(StateTrieNodes),
        table_info!(StorageTriesNodes),
        table_info!(CanonicalBlockHashes),
        table_info!(Payloads),
        table_info!(PendingBlocks),
        table_info!(SnapState),
        table_info!(StateSnapShot),
        table_info!(StorageSnapShot),
        table_info!(StorageHealPaths),
        table_info!(InvalidAncestors),
    ]
    .into_iter()
    .collect();
    let path = path.map(|p| p.as_ref().to_path_buf());
    let options = DatabaseOptions {
        page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
        mode: Mode::ReadWrite(ReadWriteOptions {
            max_size: Some(MAX_MAP_SIZE),
            ..Default::default()
        }),
        ..Default::default()
    };
    Database::create_with_options(path, options, &tables)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlp::TupleRLP;
    use bytes::Bytes;
    use ethrex_common::{
        Address, H256,
        types::{BlockHash, Index, Log, TxType},
    };

    #[test]
    fn mdbx_smoke_test() {
        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( Example ) String => String
        );

        // Assemble database chart
        let tables = [table_info!(Example)].into_iter().collect();

        let key = "Hello".to_string();
        let value = "World!".to_string();

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<Example>(key.clone(), value.clone()).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<Example>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_structs_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey([u8; 32]);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 32];

            fn encode(self) -> Self::Encoded {
                Encodable::encode(self.0)
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( StructsExample ) ExampleKey => ExampleValue
        );

        // Assemble database chart
        let tables = [table_info!(StructsExample)].into_iter().collect();
        let key = ExampleKey([151; 32]);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<StructsExample>(key, value).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<StructsExample>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_dupsort_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey(u8);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 1];

            fn encode(self) -> Self::Encoded {
                [self.0]
            }
        }
        impl Decodable for ExampleKey {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                if b.len() != 1 {
                    anyhow::bail!("Invalid length");
                }
                Ok(Self(b[0]))
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        dupsort!(
            /// Example table.
            ( DupsortExample ) ExampleKey => (ExampleKey, ExampleValue) [ExampleKey]
        );

        // Assemble database chart
        let tables = [table_info!(DupsortExample)].into_iter().collect();
        let key = ExampleKey(151);
        let subkey1 = ExampleKey(16);
        let subkey2 = ExampleKey(42);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<DupsortExample>(key, (subkey1, value)).unwrap();
            txn.upsert::<DupsortExample>(key, (subkey2, value)).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        {
            let txn = db.begin_read().unwrap();
            let mut cursor = txn.cursor::<DupsortExample>().unwrap();
            let value1 = cursor.seek_exact(key).unwrap().unwrap();
            assert_eq!(value1, (key, (subkey1, value)));
            let value2 = cursor.seek_value(key, subkey2).unwrap().unwrap();
            assert_eq!(value2, (subkey2, value));
        };

        // Walk through duplicates
        {
            let txn = db.begin_read().unwrap();
            let cursor = txn.cursor::<DupsortExample>().unwrap();
            let mut acc = 0;
            for key in cursor.walk_key(key, None).map(|r| r.unwrap().0.0) {
                acc += key;
            }

            assert_eq!(acc, 58);
        }
    }

    // Test IndexedChunks implementation with receipts as the type
    #[test]
    fn mdbx_indexed_chunks_test() {
        dupsort!(
            /// Receipts table.
            ( Receipts ) TupleRLP<BlockHash, Index>[Index] => IndexedChunk<Receipt>
        );

        let tables = [table_info!(Receipts)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let db = Database::create_with_options(None, options, &tables).unwrap();

        let mut receipts = vec![];
        for i in 0..10 {
            receipts.push(generate_big_receipt(100 * (i + 1), 10, 10 * (i + 1)));
        }

        // encode receipts
        let block_hash = H256::random();
        let mut key_values = vec![];
        for (i, receipt) in receipts.iter().enumerate() {
            let key = (block_hash, i as u64).into();
            let receipt_rlp = receipt.encode_to_vec();
            let Some(mut entries) = IndexedChunk::from::<Receipts>(key, &receipt_rlp) else {
                continue;
            };
            key_values.append(&mut entries);
        }

        // store values
        let txn = db.begin_readwrite().unwrap();
        let mut cursor = txn.cursor::<Receipts>().unwrap();
        for (key, value) in key_values {
            cursor.upsert(key, value).unwrap()
        }
        txn.commit().unwrap();

        // now retrieve the values and assert they are the same
        let mut stored_receipts = vec![];
        let mut receipt_index = 0;
        let mut key: TupleRLP<BlockHash, Index> = (block_hash, 0).into();
        let txn = db.begin_read().unwrap();
        let mut cursor = txn.cursor::<Receipts>().unwrap();
        while let Some(receipt) = IndexedChunk::read_from_db(&mut cursor, key).unwrap() {
            stored_receipts.push(receipt);
            receipt_index += 1;
            key = (block_hash, receipt_index).into();
        }

        assert_eq!(receipts, stored_receipts);
    }

    // This test verifies the 256-chunk-per-value limitation on indexed chunks.
    // Given a value size of 2022 bytes, we can store up to 256 * 2022 = 517,632 - 256 bytes.
    // The 256 subtraction accounts for the index byte overhead.
    // We expect that exceeding this storage limit results in a `None` when writing.
    #[test]
    fn indexed_chunk_storage_limit_exceeded() {
        dupsort!(
            /// example table.
            ( Example ) BlockHashRLP[Index] => IndexedChunk<Vec<u8>>
        );

        let tables = [table_info!(Example)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let _ = Database::create_with_options(None, options, &tables).unwrap();

        let block_hash = H256::random();

        // we want to store the maximum
        let max_data_bytes: usize = 517377;
        let data = Bytes::from(vec![1u8; max_data_bytes]);
        let key = block_hash.into();
        let entries = IndexedChunk::<Vec<u8>>::from::<Example>(key, &data);

        assert!(entries.is_none());
    }

    // This test verifies the 256-chunk-per-value limitation on indexed chunks.
    // Given a value size of 2022 bytes, we can store up to 256 * 2022 = 517,632 - 256 bytes.
    // The 256 subtraction accounts for the index byte overhead.
    // We expect that we can write up to that storage limit.
    #[test]
    fn indexed_chunk_storage_store_max_limit() {
        dupsort!(
            /// example table.
            ( Example ) BlockHashRLP[Index] => IndexedChunk<Vec<u8>>
        );

        let tables = [table_info!(Example)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let db = Database::create_with_options(None, options, &tables).unwrap();

        let block_hash = H256::random();

        // we want to store the maximum
        let max_data_bytes: usize = 517376;
        let data = Bytes::from(vec![1u8; max_data_bytes]);
        let key = block_hash.into();
        let entries = IndexedChunk::<Vec<u8>>::from::<Example>(key, &data).unwrap();

        // store values
        let txn = db.begin_readwrite().unwrap();
        let mut cursor = txn.cursor::<Example>().unwrap();
        for (k, v) in entries {
            cursor.upsert(k, v).unwrap();
        }
        txn.commit().unwrap();
    }

    fn generate_big_receipt(
        data_size_in_bytes: usize,
        logs_size: usize,
        topics_size: usize,
    ) -> Receipt {
        let large_data: Bytes = Bytes::from(vec![1u8; data_size_in_bytes]);
        let large_topics: Vec<H256> = std::iter::repeat_n(H256::random(), topics_size).collect();

        let logs = std::iter::repeat_n(
            Log {
                address: Address::random(),
                topics: large_topics.clone(),
                data: large_data.clone(),
            },
            logs_size,
        )
        .collect();

        Receipt {
            tx_type: TxType::EIP7702,
            succeeded: true,
            cumulative_gas_used: u64::MAX,
            logs,
        }
    }
}
