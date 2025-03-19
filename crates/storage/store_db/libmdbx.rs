use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
    BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, Rlp, TransactionHashRLP, TupleRLP,
};
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::trie_db::libmdbx::LibmdbxTrieDB;
use crate::trie_db::libmdbx_dupsort::LibmdbxDupsortTrieDB;
use crate::utils::{ChainDataIndex, SnapStateIndex};
use anyhow::Result;
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    payload::PayloadBundle, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
    ChainConfig, Index, Receipt, Transaction,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie};
use libmdbx::orm::{Decodable, DupSort, Encodable, Table};
use libmdbx::{
    dupsort,
    orm::{table, Database},
    table_info,
};
use libmdbx::{DatabaseOptions, Mode, PageSize, ReadWriteOptions, TransactionKind};
use serde_json;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Arc;

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        Ok(Self {
            db: Arc::new(init_db(Some(path))),
        })
    }

    // Helper method to write into a libmdbx table
    fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        txn.upsert::<T>(key, value)
            .map_err(StoreError::LibmdbxError)?;
        txn.commit().map_err(StoreError::LibmdbxError)
    }

    // Helper method to write into a libmdbx table in batch
    fn write_batch<T: Table>(
        &self,
        key_values: impl Iterator<Item = (T::Key, T::Value)>,
    ) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;

        let mut cursor = txn.cursor::<T>().map_err(StoreError::LibmdbxError)?;
        for (key, value) in key_values {
            cursor
                .upsert(key, value)
                .map_err(StoreError::LibmdbxError)?;
        }
        txn.commit().map_err(StoreError::LibmdbxError)
    }

    // Helper method to read from a libmdbx table
    fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        txn.get::<T>(key).map_err(StoreError::LibmdbxError)
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        Ok(self.read::<CanonicalBlockHashes>(number)?.map(|a| a.to()))
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write::<Headers>(block_hash.into(), block_header.into())
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let hashes_and_headers = block_hashes
            .into_iter()
            .zip(block_headers)
            .map(|(hash, header)| (hash.into(), header.into()));
        self.write_batch::<Headers>(hashes_and_headers)
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            Ok(self.read::<Headers>(hash.into())?.map(|b| b.to()))
        } else {
            Ok(None)
        }
    }

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write::<Bodies>(block_hash.into(), block_body.into())
    }

    fn add_block(&self, block: Block) -> std::result::Result<(), StoreError> {
        self.add_blocks(&[block])
    }

    fn add_blocks(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let tx = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;

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
    }

    fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values = blocks
            .iter()
            .map(|e| (e.header.number.into(), e.hash().into()));

        self.write_batch::<CanonicalBlockHashes>(key_values)
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
        Ok(self.read::<Bodies>(block_hash.into())?.map(|b| b.to()))
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        Ok(self.read::<Headers>(block_hash.into())?.map(|b| b.to()))
    }

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<BlockNumbers>(block_hash.into(), block_number)
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        self.read::<BlockNumbers>(block_hash.into())
    }

    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.write::<AccountCodes>(code_hash.into(), code.into())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        Ok(self.read::<AccountCodes>(code_hash.into())?.map(|b| b.to()))
    }

    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key: Rlp<(BlockHash, Index)> = (block_hash, index).into();
        let Some(entries) = IndexedChunk::from::<Receipts>(key, &receipt.encode_to_vec()) else {
            return Err(StoreError::Custom("Invalid size".to_string()));
        };
        self.write_batch::<Receipts>(entries.into_iter())
    }

    fn get_receipt(
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

    fn add_transaction_location(
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
    }

    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<TransactionLocations>()
            .map_err(StoreError::LibmdbxError)?;
        Ok(cursor
            .walk_key(transaction_hash.into(), None)
            .map_while(|res| res.ok().map(|t| t.to()))
            .find(|(number, hash, _index)| {
                self.get_block_hash_by_block_number(*number)
                    .is_ok_and(|o| o == Some(*hash))
            }))
    }

    /// Stores the chain config serialized as json
    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::EarliestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::FinalizedBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::SafeBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::LatestBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::PendingBlockNumber)? {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        let db = Box::new(LibmdbxDupsortTrieDB::<StorageTriesNodes, [u8; 32]>::new(
            self.db.clone(),
            hashed_address.0,
        ));
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        let db = Box::new(LibmdbxTrieDB::<StateTrieNodes>::new(self.db.clone()));
        Trie::open(db, state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        self.write::<CanonicalBlockHashes>(number, hash.into())
    }

    fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<CanonicalBlockHashes>(number)
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, PayloadBundle::from_block(block).into())
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        let r = self.read::<Payloads>(payload_id)?;
        Ok(r.map(|b| b.to()))
    }

    fn update_payload(&self, payload_id: u64, payload: PayloadBundle) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, payload.into())
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash)? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index)
    }

    fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash)? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        self.db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?
            .delete::<CanonicalBlockHashes>(number, None)
            .map(|_| ())
            .map_err(StoreError::LibmdbxError)
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write::<PendingBlocks>(block.header.compute_block_hash().into(), block.into())
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        Ok(self
            .read::<PendingBlocks>(block_hash.into())?
            .map(|b| b.to()))
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        #[allow(clippy::type_complexity)]
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (tx_hash.into(), (block_number, block_hash, index).into())
            });

        self.write_batch::<TransactionLocations>(key_values)
    }

    fn add_receipts(
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

        self.write_batch::<Receipts>(key_values.into_iter())
    }

    fn add_receipts_for_blocks(
        &self,
        receipts: HashMap<BlockHash, Vec<Receipt>>,
    ) -> Result<(), StoreError> {
        let mut key_values = vec![];

        for (block_hash, receipts) in receipts.into_iter() {
            for (index, receipt) in receipts.into_iter().enumerate() {
                let key = (block_hash, index as u64).into();
                let receipt_rlp = receipt.encode_to_vec();
                let Some(mut entries) = IndexedChunk::from::<Receipts>(key, &receipt_rlp) else {
                    continue;
                };

                key_values.append(&mut entries);
            }
        }

        self.write_batch::<Receipts>(key_values.into_iter())
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

    fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint,
            block_hash.encode_to_vec(),
        )
    }

    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint)?
            .map(|ref h| BlockHash::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieKeyCheckpoint,
            last_keys.to_vec().encode_to_vec(),
        )
    }

    fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint)?
            .map(|ref c| {
                <Vec<H256>>::decode(c)?
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)
            })
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(SnapStateIndex::StorageHealPaths, accounts.encode_to_vec())
    }

    fn get_storage_heal_paths(&self) -> Result<Option<Vec<(H256, Vec<Nibbles>)>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StorageHealPaths)?
            .map(|ref h| <Vec<(H256, Vec<Nibbles>)>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn is_synced(&self) -> Result<bool, StoreError> {
        match self.read::<ChainData>(ChainDataIndex::IsSynced)? {
            None => Err(StoreError::Custom("Sync status not found".to_string())),
            Some(ref rlp) => RLPDecode::decode(rlp).map_err(|_| StoreError::DecodeError),
        }
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        self.write::<ChainData>(ChainDataIndex::IsSynced, status.encode_to_vec())
    }

    fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.write::<SnapState>(SnapStateIndex::StateHealPaths, paths.encode_to_vec())
    }

    fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateHealPaths)?
            .map(|ref h| <Vec<Nibbles>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn clear_snap_state(&self) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        txn.clear_table::<SnapState>()
            .map_err(StoreError::LibmdbxError)?;
        txn.commit().map_err(StoreError::LibmdbxError)
    }

    fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        self.write_batch::<StateSnapShot>(
            account_hashes
                .into_iter()
                .map(|h| h.into())
                .zip(account_states.into_iter().map(|a| a.into())),
        )
    }

    fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;

        for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
            txn.upsert::<StorageSnapShot>(account_hash.into(), (key.into(), value.into()))
                .map_err(StoreError::LibmdbxError)?;
        }

        txn.commit().map_err(StoreError::LibmdbxError)
    }

    fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieRebuildCheckpoint,
            (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec(),
        )
    }

    fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let Some((root, checkpoints)) = self
            .read::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint)?
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

    fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StorageTrieRebuildPending,
            pending.encode_to_vec(),
        )
    }

    fn get_storage_trie_rebuild_pending(&self) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StorageTrieRebuildPending)?
            .map(|ref h| <Vec<(H256, H256)>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    fn clear_snapshot(&self) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        txn.clear_table::<StateSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        txn.clear_table::<StorageSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        txn.commit().map_err(StoreError::LibmdbxError)?;
        Ok(())
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StateSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        let iter = cursor
            .walk(Some(start.into()))
            .map_while(|res| res.ok().map(|(hash, acc)| (hash.to(), acc.to())))
            .take(MAX_SNAPSHOT_READS);
        Ok(iter.collect::<Vec<_>>())
    }

    fn read_storage_snapshot(
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

// In order to use NodeHash as key in a dupsort table we must encode it into a fixed size type
pub fn node_hash_to_fixed_size(node_hash: Vec<u8>) -> [u8; 33] {
    // keep original len so we can re-construct it later
    let original_len = node_hash.len();
    // original len will always be lower or equal to 32 bytes
    debug_assert!(original_len <= 32);
    // Pad the node_hash with zeros to make it fixed_size (in case of inline)
    let mut node_hash = node_hash;
    node_hash.resize(32, 0);
    // Encode the node as [original_len, node_hash...]
    std::array::from_fn(|i| match i {
        0 => original_len as u8,
        n => node_hash[n - 1],
    })
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
    ( StateTrieNodes ) Vec<u8> => Vec<u8>
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

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> Database {
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
    ]
    .into_iter()
    .collect();
    let path = path.map(|p| p.as_ref().to_path_buf());
    let options = DatabaseOptions {
        page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
        mode: Mode::ReadWrite(ReadWriteOptions {
            // Set max DB size to 1TB
            max_size: Some(1024_isize.pow(4)),
            ..Default::default()
        }),
        ..Default::default()
    };
    Database::create_with_options(path, options, &tables).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlp::TupleRLP;
    use bytes::Bytes;
    use ethrex_common::{
        types::{BlockHash, Index, Log, TxType},
        Address, Bloom, H256,
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
            for key in cursor.walk_key(key, None).map(|r| r.unwrap().0 .0) {
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
                max_size: Some(1024_isize.pow(4)),
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
                max_size: Some(1024_isize.pow(4)),
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
                max_size: Some(1024_isize.pow(4)),
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
        let large_topics: Vec<H256> = std::iter::repeat(H256::random())
            .take(topics_size)
            .collect();

        let logs = std::iter::repeat(Log {
            address: Address::random(),
            topics: large_topics.clone(),
            data: large_data.clone(),
        })
        .take(logs_size)
        .collect();

        Receipt {
            tx_type: TxType::EIP7702,
            succeeded: true,
            cumulative_gas_used: u64::MAX,
            bloom: Bloom::default(),
            logs,
        }
    }
}
