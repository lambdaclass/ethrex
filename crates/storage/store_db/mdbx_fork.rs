use ethrex_trie::{InMemoryTrieDB, NodeHash};
use reth_db::table::DupSort;
use reth_primitives::revm_primitives::db::components::block_hash;
use reth_provider::providers::StaticFileProvider;
use serde_json::value;
use std::cell::{Cell, LazyCell, OnceCell};
use std::iter;
use std::marker::PhantomData;
use std::ops::Div;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
// Storage implementation using reth's fork of libmdbx
// to compare against our own.
use reth_provider::{DatabaseProvider, StaticFileAccess};
use std::sync::{LazyLock, Mutex};

use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
    BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, Rlp, TransactionHashRLP, TupleRLP,
};
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::utils::{ChainDataIndex, SnapStateIndex};
use alloy_consensus::{Header, Sealed};
use alloy_eips::eip4895::Withdrawal as RethWithdrawal;
use alloy_primitives::{Bytes as AlloyBytes, B256};
use anyhow::{Chain, Result};
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::Withdrawal;
use ethrex_common::types::{
    payload::PayloadBundle, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
    ChainConfig, Index, Receipt, Transaction,
};
use ethrex_common::{Bloom, H160};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie, TrieDB, TrieError};
use reth_blockchain_tree_api::BlockValidationKind;
use reth_chainspec::ChainSpec;
use reth_db::mdbx::{init_db, DatabaseArguments, DatabaseEnv};
use reth_db::AccountsTrie;
use reth_db::{tables, StoragesTrie};
use reth_db::{
    transaction::{DbTx, DbTxMut},
    Database,
};
use reth_db_api::cursor::DbCursorRO;
use reth_db_api::cursor::DbCursorRW;
use reth_db_api::cursor::DbDupCursorRO;
use reth_db_api::cursor::DbDupCursorRW;
use reth_primitives::{
    BlockBody as RethBlockBody, Bytecode, SealedBlock, SealedBlockWithSenders, TransactionSigned,
    Withdrawals,
};
use reth_primitives_traits::SealedHeader;
use reth_provider::BlockWriter;
use reth_storage_api::DBProvider;
use std::collections::{BTreeMap, HashMap};

const DB_DUPSORT_MAX_SIZE: OnceCell<usize> = OnceCell::new();

pub struct MDBXFork {
    env: DatabaseEnv,
    account_trie: Arc<MDBXTrieDupsort<AccountTrie>>,
    storage_trie: Arc<MDBXTrieDB<StorageTrie>>,
}

impl std::fmt::Debug for MDBXFork {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        todo!()
    }
}

impl MDBXFork {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        let client_version = Default::default();
        let db_args = DatabaseArguments::new(client_version);
        let env = init_db(path, db_args).expect("Failed to initialize MDBX Fork");
        // https://libmdbx.dqdkfa.ru/intro.html#autotoc_md5
        // Value size: minimum 0, maximum 2146435072 (0x7FF00000) bytes for maps,
        // ≈½ pagesize for multimaps (2022 bytes for default 4K pagesize,
        // 32742 bytes for 64K pagesize).
        DB_DUPSORT_MAX_SIZE.get_or_init(|| page_size::get().div(2));

        let tx = env.begin_rw_txn().unwrap();

        tx.create_db(Some("AccountTrie"), DatabaseFlags::DUP_SORT)
            .unwrap();
        tx.create_db(Some("Receipts"), DatabaseFlags::DUP_SORT)
            .unwrap();

        tx.create_db(Some("StorageTrie"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("TransactionLocations"), DatabaseFlags::DUP_SORT)
            .unwrap();
        tx.create_db(Some("Bodies"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("Headers"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("BlockNumbers"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("PendingBlocks"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("CanonicalBlockHashes"), DatabaseFlags::default())
            .unwrap();

        tx.create_db(Some("ChainData"), DatabaseFlags::default())
            .unwrap();

        tx.create_db(Some("SnapState"), DatabaseFlags::default())
            .unwrap();

        tx.create_db(Some("Payloads"), DatabaseFlags::default())
            .unwrap();

        tx.create_db(Some("StateSnapShot"), DatabaseFlags::DUP_SORT)
            .unwrap();

        tx.create_db(Some("StorageSnapShot"), DatabaseFlags::DUP_SORT)
            .unwrap();

        tx.create_db(Some("StorageHealPaths"), DatabaseFlags::DUP_SORT)
            .unwrap();

        tx.commit().unwrap();

        let env_account_trie = DatabaseEnv::open(
            Path::new("/tmp/account_trie"),
            reth_db::DatabaseEnvKind::RW,
            Default::default(),
        )
        .unwrap();
        let env_storage_trie = DatabaseEnv::open(
            Path::new("/tmp/storage_trie"),
            reth_db::DatabaseEnvKind::RW,
            Default::default(),
        )
        .unwrap();
        let account_trie = Arc::new(MDBXTrieDupsort::new(env_account_trie));
        let storage_trie = Arc::new(MDBXTrieDB::new(env_storage_trie));

        Ok(Self {
            env,
            account_trie,
            storage_trie,
        })
    }
}

use reth_db_api::table::Table as RethTable;
use reth_libmdbx::{DatabaseFlags, Environment};

pub struct MDBXTrieDB<T: RethTable> {
    db: DatabaseEnv,
    phantom: PhantomData<T>,
}

impl<T> MDBXTrieDB<T>
where
    T: RethTable,
{
    pub fn new(db: DatabaseEnv) -> Self {
        let tx = db.begin_rw_txn().unwrap();
        tx.create_db(Some(T::NAME), DatabaseFlags::default())
            .unwrap();
        tx.commit().unwrap();
        Self {
            db,
            phantom: PhantomData,
        }
    }
}

impl<T> TrieDB for MDBXTrieDB<T>
where
    T: RethTable<Key = Vec<u8>, Value = Vec<u8>>,
{
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().unwrap();
        let node_hash_bytes = key.as_ref().clone().to_vec();
        Ok(tx.get::<T>(node_hash_bytes).unwrap())
    }

    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let node_hash_bytes = key.as_ref().clone().to_vec();
        tx.put::<T>(node_hash_bytes, value).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.tx_mut().unwrap();
        for (k, v) in key_values {
            let node_hash_bytes = k.as_ref().clone().to_vec();
            txn.put::<T>(node_hash_bytes, v).unwrap();
        }
        txn.commit().unwrap();
        Ok(())
    }
}

pub struct MDBXTrieDupsort<T: DupSort> {
    db: DatabaseEnv,
    phantom: PhantomData<T>,
    pub fixed_key: Arc<Mutex<Option<Vec<u8>>>>,
}

impl<T> MDBXTrieDupsort<T>
where
    T: DupSort,
{
    pub fn new(db: DatabaseEnv) -> Self {
        let tx = db.begin_rw_txn().unwrap();
        tx.create_db(Some(T::NAME), DatabaseFlags::DUP_SORT)
            .unwrap();
        tx.commit().unwrap();
        Self {
            fixed_key: Default::default(),
            db,
            phantom: PhantomData,
        }
    }
}

impl<T> TrieDB for MDBXTrieDupsort<T>
where
    T: DupSort<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>,
{
    fn get(&self, subkey: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().unwrap();
        let mut cursor = tx.cursor_dup_read::<T>().unwrap();
        let node_hash_bytes = subkey.as_ref().to_vec();
        let value = cursor
            .seek_by_key_subkey(
                self.fixed_key.lock().unwrap().as_ref().unwrap().clone(),
                node_hash_bytes,
            )
            .unwrap();

        Ok(value)
    }

    fn put(&self, subkey: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<T>().unwrap();

        cursor
            .seek_exact(self.fixed_key.lock().unwrap().as_ref().unwrap().clone())
            .unwrap();

        cursor.append_dup(subkey.as_ref().clone().to_vec(), value).unwrap();

        tx.commit().unwrap();
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<T>().unwrap();

        // Position at main key once
        cursor
            .seek_exact(self.fixed_key.lock().unwrap().as_ref().unwrap().clone())
            .unwrap();

        for (subkey, value) in key_values {
            // Append each subkey+value pair
            cursor.append_dup(subkey.as_ref().clone().to_vec(), value).unwrap();
        }

        tx.commit().unwrap();
        Ok(())
    }
}

use reth_db::TableType;
use reth_db::TableViewer;
use std::fmt::{self, Error, Formatter};

tables! {
    table AccountTrie<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table Receipts<Key = Vec<u8>, Value = Vec<u8>, SubKey = u64>;
    table StorageTrie<Key = Vec<u8>, Value = Vec<u8>>;
    table TransactionLocations<Key = Vec<u8>, Value = Vec<u8>>;
    table Bodies<Key = Vec<u8>, Value = Vec<u8>>;
    table Headers<Key = Vec<u8>, Value = Vec<u8>>;
    table CanonicalBlockHashes<Key = u64, Value = Vec<u8>>;
    table BlockNumbers<Key = Vec<u8>, Value = u64>;
    table PendingBlocks<Key = Vec<u8>, Value = Vec<u8>>;
    table ChainData<Key = u8, Value = Vec<u8>>;
    table SnapState<Key = u8, Value = Vec<u8>>;
    table Payloads<Key = u64, Value = Vec<u8>>;
    table StateSnapShot<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table StorageSnapshot<Key = Vec<u8>, Value = Vec<u8>, SubKey = Vec<u8>>;
    table StorageHealPaths<Key = Vec<u8>, Value = Vec<u8>>;
}

#[async_trait::async_trait]
impl StoreEngine for MDBXFork {
    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        let tx = self
            .env
            .tx_mut()
            .expect("Could not start TX for block headers");
        tx.put::<Headers>(block_hash.encode_to_vec(), block_header.encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let tx = self
            .env
            .tx_mut()
            .expect("Could not start tx for block headers (batched)");
        for (header, hash) in block_headers.into_iter().zip(block_hashes) {
            tx.put::<Headers>(hash.encode_to_vec(), header.encode_to_vec())
                .unwrap();
        }

        tx.commit().unwrap();
        Ok(())
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let tx = self.env.tx().expect("Could not start tx for block headers");
        let Some(header_hash) = tx.get::<CanonicalBlockHashes>(block_number).unwrap() else {
            return Ok(None);
        };
        let header = tx
            .get::<Headers>(header_hash)
            .unwrap()
            .map(|h| BlockHeader::decode(h.as_ref()).unwrap());
        Ok(header)
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        let key = block_hash.encode_to_vec();
        let body = block_body.encode_to_vec();
        tx.put::<Bodies>(key, body).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        for block in blocks {
            let number = block.header.number;
            let hash = block.hash();
            for (index, transaction) in block.body.transactions.iter().enumerate() {
                tx.put::<TransactionLocations>(
                    transaction.compute_hash().0.into(),
                    (number, hash, index as u64).encode_to_vec(),
                )
                .unwrap();
            }

            tx.put::<Bodies>(hash.encode_to_vec(), block.body.encode_to_vec())
                .unwrap();
            tx.put::<Headers>(hash.encode_to_vec(), block.header.encode_to_vec())
                .unwrap();
            tx.put::<BlockNumbers>(hash.encode_to_vec(), number)
                .unwrap();
        }
        tx.commit().unwrap();
        Ok(())
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values: Vec<_> = blocks
            .iter()
            .map(|e| (e.header.number, e.hash().encode_to_vec()))
            .collect();
        let tx = self.env.tx_mut().unwrap();
        for (k, v) in key_values {
            tx.put::<CanonicalBlockHashes>(k, v).unwrap();
        }
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_block_body(&self, block_number: BlockNumber) -> Result<Option<BlockBody>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(hash) = tx.get::<CanonicalBlockHashes>(block_number).unwrap() else {
            return Ok(None);
        };
        let Some(encoded_body) = tx.get::<Bodies>(hash).unwrap() else {
            return Ok(None);
        };
        let decoded = BlockBody::decode(&encoded_body).unwrap();
        Ok(Some(decoded))
    }

    async fn get_block_bodies(&self, from: BlockNumber, to: BlockNumber) -> Result<Vec<BlockBody>, StoreError> {
        let mut encoded_bodies = vec![];
        {
            let tx = self.env.tx().unwrap();
            let mut hashes_cursor = tx.cursor_read::<CanonicalBlockHashes>().unwrap();
            let iterator = hashes_cursor.walk_range(from..=to).unwrap();
            for next in iterator {
                let Ok((_, encoded_hash)) = next else {
                    break;
                };
                let Ok(Some(encoded_body)) = tx.get::<Bodies>(encoded_hash) else {
                    break;
                };
                encoded_bodies.push(encoded_body)
            }
        }
        let bodies: Vec<_> = encoded_bodies.into_iter().map(|eb| RLPDecode::decode(&eb).unwrap()).collect();
        Ok(bodies)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let mut encoded_result = vec![];
        {
            let encoded_hashes: Vec<Vec<u8>> = hashes.into_iter().map(|h| RLPEncode::encode_to_vec(&h)).collect();
            let tx = self.env.tx().unwrap();
            for eh in encoded_hashes {
                let Some(encoded_body) = tx.get::<Bodies>(eh).unwrap() else {
                    break;
                };
                encoded_result.push(encoded_body);
            }
        }
        let bodies: Result<Vec<_>, _> = encoded_result.into_iter().map(|encoded| RLPDecode::decode(&encoded)).collect();
        Ok(bodies.unwrap())
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        let tx = self.env.tx_mut().unwrap();
        let res: Vec<(H256, Vec<Nibbles>)> = tx
            .cursor_read::<StorageHealPaths>()
            .unwrap()
            .walk(None)
            .unwrap()
            .take(limit)
            .map(|fetched| {
                let (encoded_h, encoded_nibbles) = fetched.unwrap();
                let hash = H256::decode(&encoded_h).unwrap();
                let nibbles = <Vec<Nibbles>>::decode(&encoded_nibbles).unwrap();
                (hash, nibbles)
            })
            .collect();
        for (hash, _) in res.iter() {
            tx.delete::<StorageHealPaths>(hash.encode_to_vec(), None).unwrap();
        }
        Ok(res)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        let encoded_hash = block_hash.encode_to_vec();
        let tx = self.env.tx().unwrap();
        let Some(encoded_body) = tx.get::<Bodies>(encoded_hash).unwrap() else {
            return Ok(None);
        };
        let decoded = BlockBody::decode(&encoded_body).unwrap();
        Ok(Some(decoded))
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let tx = self.env.tx().unwrap();
        let header = tx
            .get::<Headers>(block_hash.encode_to_vec())
            .unwrap()
            .map(|header| BlockHeader::decode(&header[..]).unwrap());
        Ok(header)
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        let encoded_hash = block_hash.encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<BlockNumbers>(encoded_hash, block_number).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        let encoded_key = block_hash.encode_to_vec();
        let tx = self.env.tx().unwrap();
        Ok(tx.get::<BlockNumbers>(encoded_key).unwrap())
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        let key: B256 = code_hash.0.into();
        let code = reth_primitives_traits::Bytecode::new_raw(AlloyBytes(code));
        let tx = self
            .env
            .tx_mut()
            .expect("could not start tx for account code");
        tx.put::<tables::Bytecodes>(key, code).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        let tx = self
            .env
            .tx()
            .expect("could not start tx to get account code");
        let Ok(code) = tx.get::<tables::Bytecodes>(code_hash.0.into()) else {
            panic!("Failed to fetch bytecode from db")
        };
        Ok(code.map(|bytecode: Bytecode| -> Bytes { bytecode.bytes().into() }))
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let encoded = receipt.encode_to_vec();
        let main_key = block_hash.encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<Receipts>().unwrap();
        if encoded.len() > *DB_DUPSORT_MAX_SIZE.get().unwrap() {
            let chunks: Vec<Vec<u8>> = encoded
                .chunks(*DB_DUPSORT_MAX_SIZE.get().unwrap())
                .map(|chunk| chunk.to_vec())
                .collect();

            for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                let mut chunked_value = vec![];

                chunked_value.extend_from_slice(&(index as u64).to_be_bytes());

                chunked_value.push(chunk_index as u8);

                chunked_value.extend_from_slice(&chunk);

                cursor.append_dup(main_key.clone(), chunked_value).unwrap();
            }
        } else {
            let mut value = Vec::with_capacity(8 + 1 + encoded.len());

            value.extend_from_slice(&(index as u64).to_be_bytes());

            value.push(0u8);

            value.extend_from_slice(&encoded);

            cursor.append_dup(main_key.clone(), value).unwrap();
        }
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_receipt(
        &self,
        block_number: u64,
        receipt_index: u64,
    ) -> Result<Option<Receipt>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(hash) = tx.get::<CanonicalBlockHashes>(block_number).unwrap() else {
            return Ok(None);
        };

        let key = hash.encode_to_vec();

        let mut cursor = tx.cursor_dup_read::<Receipts>().unwrap();

        if cursor.seek_exact(key.clone()).unwrap().is_none() {
            return Ok(None);
        }

        let mut chunks = Vec::new();

        while let Some((k, v)) = cursor.current().unwrap() {
            if k != key {
                break;
            }

            let mut index_bytes = [0u8; 8];
            index_bytes.copy_from_slice(&v[0..8]);
            let stored_index = u64::from_be_bytes(index_bytes);

            if stored_index == receipt_index {
                let chunk_index = v[8] as usize;
                let chunk_data = v[9..].to_vec();

                chunks.push((chunk_index, chunk_data));
            }

            if cursor.next_dup().unwrap().is_none() {
                break;
            }
        }

        if chunks.is_empty() {
            return Ok(None);
        }

        chunks.sort_by_key(|(idx, _)| *idx);

        let mut complete_data = Vec::new();
        for (_, chunk_data) in chunks {
            complete_data.extend_from_slice(&chunk_data);
        }

        let receipt: Receipt = RLPDecode::decode(&complete_data).unwrap();
        Ok(Some(receipt))
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        let value = (block_number, block_hash, index).encode_to_vec();
        let key = transaction_hash.encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<TransactionLocations>(key, value).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let key = transaction_hash.encode_to_vec();
        let tx = self.env.tx().unwrap();
        let Some(encoded_tuple) = tx.get::<TransactionLocations>(key).unwrap() else {
            return Ok(None);
        };
        let decoded = <(BlockNumber, BlockHash, Index)>::decode(&encoded_tuple).unwrap();
        Ok(Some(decoded))
    }

    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::ChainConfig as u8,
            serde_json::to_string(chain_config).unwrap().into_bytes(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        let tx = self.env.tx().unwrap();
        match tx
            .get::<ChainData>(ChainDataIndex::ChainConfig as u8)
            .unwrap()
        {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    async fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::EarliestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx
            .get::<ChainData>(ChainDataIndex::EarliestBlockNumber as u8)
            .unwrap()
            .map(|r| RLPDecode::decode(r.as_ref()).unwrap());
        Ok(res)
    }

    async fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        Ok(tx
            .get::<ChainData>(ChainDataIndex::FinalizedBlockNumber as u8)
            .unwrap()
            .map(|ref bn| BlockNumber::decode(bn).unwrap()))
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::SafeBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        Ok(tx
            .get::<ChainData>(ChainDataIndex::SafeBlockNumber as u8)
            .unwrap()
            .map(|ref num| BlockNumber::decode(num).unwrap()))
    }

    async fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::LatestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx
            .get::<ChainData>(ChainDataIndex::LatestBlockNumber as u8)
            .unwrap()
            .map(|r| RLPDecode::decode(r.as_ref()).unwrap());
        Ok(res)
    }

    async fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let encoded = block_number.encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(ChainDataIndex::PendingBlockNumber as u8, encoded)
            .unwrap();
        Ok(())
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(res) = tx
            .get::<ChainData>(ChainDataIndex::PendingBlockNumber as u8)
            .unwrap()
        else {
            return Ok(None);
        };
        let decoded = BlockNumber::decode(&res).unwrap();
        Ok(Some(decoded))
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        *(self.account_trie.fixed_key.lock().unwrap()) = Some(hashed_address.0.as_slice().to_vec());
        Trie::open(self.account_trie.clone(), storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        Trie::open(self.storage_trie.clone(), state_root)
    }

    async fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<CanonicalBlockHashes>(number, hash.encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        let tx = self.env.tx().unwrap();
        let bytes = tx.get::<CanonicalBlockHashes>(number).unwrap();

        match bytes {
            Some(bytes) => {
                let hash: BlockHash = RLPDecode::decode(bytes.as_ref()).unwrap();
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<Payloads>(payload_id, PayloadBundle::from_block(block).encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx.get::<Payloads>(payload_id).unwrap();
        match res {
            Some(encoded) => Ok(Some(PayloadBundle::decode(&encoded[..]).unwrap())),
            None => Ok(None),
        }
    }

    async fn update_payload(&self, payload_id: u64, payload: PayloadBundle) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<Payloads>(payload_id, payload.encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
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
        let tx = self.env.tx().unwrap();
        let body = tx.get::<Bodies>(block_hash.encode_to_vec()).unwrap();
        let header = tx.get::<Headers>(block_hash.encode_to_vec()).unwrap();
        match (body, header) {
            (Some(body), Some(header)) => {
                let body: BlockBody = RLPDecode::decode(body.as_ref()).unwrap();
                let header: BlockHeader = RLPDecode::decode(header.as_ref()).unwrap();
                let block = Block::new(header, body);
                Ok(Some(block))
            }
            _ => Ok(None),
        }
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<PendingBlocks>(
            block.header.compute_block_hash().encode_to_vec(),
            block.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let encoded_hash = block_hash.encode_to_vec();
        let tx = self.env.tx().unwrap();
        let Some(encoded_block) = tx.get::<PendingBlocks>(encoded_hash).unwrap() else {
            return Ok(None);
        };
        Ok(Some(Block::decode(&encoded_block).unwrap()))
    }

    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (
                    tx_hash.encode_to_vec(),
                    (block_number, block_hash, index).encode_to_vec(),
                )
            })
            .collect::<Vec<_>>();
        let tx = self.env.tx_mut().unwrap();
        for (k, v) in key_values {
            tx.put::<TransactionLocations>(k, v).unwrap();
        }
        Ok(())
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<Receipts>().unwrap();

        let main_key = block_hash.encode_to_vec();

        for (index, receipt) in receipts.into_iter().enumerate() {
            let receipt_bytes = receipt.encode_to_vec();

            if receipt_bytes.len() > *DB_DUPSORT_MAX_SIZE.get().unwrap() {
                let chunks: Vec<Vec<u8>> = receipt_bytes
                    .chunks(*DB_DUPSORT_MAX_SIZE.get().unwrap())
                    .map(|chunk| chunk.to_vec())
                    .collect();

                for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                    let mut chunked_value = vec![];

                    chunked_value.extend_from_slice(&(index as u64).to_be_bytes());

                    chunked_value.push(chunk_index as u8);

                    chunked_value.extend_from_slice(&chunk);

                    cursor.append_dup(main_key.clone(), chunked_value).unwrap();
                }
            } else {
                let mut value = Vec::with_capacity(8 + 1 + receipt_bytes.len());

                value.extend_from_slice(&(index as u64).to_be_bytes());

                value.push(0u8);

                value.extend_from_slice(&receipt_bytes);

                cursor.append_dup(main_key.clone(), value).unwrap();
            }
        }

        // Commit the transaction
        tx.commit().unwrap();
        Ok(())
    }
    async fn add_receipts_for_blocks(
        &self,
        receipts: std::collections::HashMap<BlockHash, Vec<Receipt>>,
    ) -> Result<(), StoreError> {
        for (block_hash, receipts) in receipts {
            self.add_receipts(block_hash, receipts).await.unwrap();
        }
        Ok(())
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        todo!()
    }

    async fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint as u8,
            block_hash.encode_to_vec(),
        )
        .unwrap();
        Ok(())
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let tx = self.env.tx().unwrap();
        Ok(tx
            .get::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint as u8)
            .unwrap()
            .map(|h| BlockHash::decode(h.as_ref()).unwrap()))
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        let encoded = last_keys.to_vec().encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint as u8, encoded)
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        let tx = self.env.tx().unwrap();
        tx.get::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint as u8)
            .unwrap()
            .map(|ref c| {
                <Vec<H256>>::decode(c)
                    .unwrap()
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)
            })
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        let encoded: Vec<_> = accounts.into_iter().map(|(hash, nibbles)| (hash.encode_to_vec(), nibbles.encode_to_vec())).collect();
        let tx = self.env.tx_mut().unwrap();
        for (k, v) in encoded {
            tx.put::<StorageHealPaths>(k, v).unwrap();
        }
        Ok(())
    }


    async fn is_synced(&self) -> Result<bool, StoreError> {
        let tx = self.env.tx().unwrap();
        let sync_status = tx
            .get::<ChainData>(ChainDataIndex::IsSynced as u8)
            .unwrap()
            .unwrap();
        Ok(RLPDecode::decode(&sync_status).unwrap())
    }

    async fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(ChainDataIndex::IsSynced as u8, status.encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        let encoded = paths.encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(SnapStateIndex::StateHealPaths as u8, encoded)
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(res) = tx
            .get::<SnapState>(SnapStateIndex::StateHealPaths as u8)
            .unwrap()
        else {
            return Ok(None);
        };
        let decoded = <Vec<Nibbles>>::decode(&res[..]).unwrap();
        Ok(Some(decoded))
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.clear::<SnapState>().unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        let key_values: Vec<_> = account_hashes
            .into_iter()
            .map(|h| h.encode_to_vec())
            .zip(account_states.into_iter().map(|a| a.encode_to_vec()))
            .collect();
        let tx = self.env.tx_mut().unwrap();
        for (k, v) in key_values {
            tx.put::<StateSnapShot>(k, v).unwrap();
        }
        tx.commit().unwrap();
        Ok(())
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let encoded_hash = account_hash.encode_to_vec();
        let encoded_values = storage_keys
            .into_iter()
            .zip(storage_values.into_iter())
            .map(|v| v.encode_to_vec())
            .collect::<Vec<_>>();
        let tx = self.env.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<StateSnapShot>().unwrap();
        cursor.seek_exact(encoded_hash.clone()).unwrap();
        for v in encoded_values {
            cursor.append_dup(encoded_hash.clone(), v).unwrap();
        }
        Ok(())
    }

    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        // Pre-encode all data before starting DB interaction
        let pre_encoded: Vec<(Vec<u8>, Vec<Vec<u8>>)> = account_hashes
            .into_iter()
            .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
            .map(|(account_hash, (keys, values))| {
                let encoded_hash = account_hash.encode_to_vec();
                let encoded_pairs = keys
                    .into_iter()
                    .zip(values.into_iter())
                    .map(|(k, v)| (k, v).encode_to_vec())
                    .collect();
                (encoded_hash, encoded_pairs)
            })
            .collect();

        // Now perform all DB operations in one quick sequence
        let tx = self.env.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<StorageSnapshot>().unwrap();

        for (encoded_hash, encoded_pairs) in pre_encoded {
            cursor.seek_exact(encoded_hash.clone()).unwrap();
            for pair in encoded_pairs {
                cursor.append_dup(encoded_hash.clone(), pair).unwrap();
            }
        }

        Ok(())
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        let encoded = (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec();
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint as u8, encoded)
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(encoded) = tx
            .get::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint as u8)
            .unwrap()
        else {
            return Ok(None);
        };
        let (root, checkpoints): (H256, Vec<H256>) = RLPDecode::decode(&encoded).unwrap();
        Ok(Some((root, checkpoints.try_into().unwrap())))
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(
            SnapStateIndex::StorageTrieRebuildPending as u8,
            pending.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    async fn get_storage_trie_rebuild_pending(&self) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        let tx = self.env.tx().unwrap();
        let Some(encoded) = tx
            .get::<SnapState>(SnapStateIndex::StorageTrieRebuildPending as u8)
            .unwrap()
        else {
            return Ok(None);
        };
        let decoded: Vec<(H256, H256)> = RLPDecode::decode(&encoded).unwrap();
        Ok(Some(decoded))
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.clear::<StateSnapShot>().unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        let key = start.encode_to_vec();
        let mut results = vec![];
        {
            let tx = self.env.tx().unwrap();
            let mut cursor = tx.cursor_read::<StateSnapShot>().unwrap();
            cursor.seek_exact(key).unwrap();
            let mut readings = 0;
            while readings < MAX_SNAPSHOT_READS {
                let Some((encoded_key, encoded_value)) = cursor.next_dup().unwrap() else {
                    break;
                };
                results.push((encoded_key, encoded_value));
                readings += 1;
            }
        }
        let results = results
            .into_iter()
            .map(|(ref encoded_k, ref encoded_v)| {
                (
                    H256::decode(encoded_k).unwrap(),
                    AccountState::decode(encoded_v).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        Ok(results)
    }

    async fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let key = start.encode_to_vec();
        let mut results = vec![];
        {
            let tx = self.env.tx().unwrap();
            let mut cursor = tx.cursor_read::<StateSnapShot>().unwrap();
            cursor.seek_exact(key).unwrap();
            let mut readings = 0;
            while readings < MAX_SNAPSHOT_READS {
                let Some((encoded_key, encoded_value)) = cursor.next_dup().unwrap() else {
                    break;
                };
                results.push((encoded_key, encoded_value));
                readings += 1;
            }
        }
        let results = results
            .into_iter()
            .map(|(ref encoded_k, ref encoded_v)| {
                (
                    H256::decode(encoded_k).unwrap(),
                    U256::decode(encoded_v).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        Ok(results)
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
) -> Result<(), StoreError> { todo!() }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> { todo!() }
}
