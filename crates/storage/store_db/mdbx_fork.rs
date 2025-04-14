use ethrex_trie::InMemoryTrieDB;
use reth_db::table::DupSort;
use reth_provider::providers::StaticFileProvider;
use std::cell::{Cell, LazyCell, OnceCell};
use std::marker::PhantomData;
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
use anyhow::Result;
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
use std::collections::HashMap;

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

        // Create tables in main environment
        let tx = env.begin_rw_txn().unwrap();

        // DupSort tables (those with SubKey)
        tx.create_db(Some("AccountTrie"), DatabaseFlags::DUP_SORT)
            .unwrap();
        tx.create_db(Some("Receipts"), DatabaseFlags::DUP_SORT)
            .unwrap();

        // Regular tables
        tx.create_db(Some("StorageTrie"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("PayloadsTable"), DatabaseFlags::default())
            .unwrap();
        tx.create_db(Some("TransactionLocations"), DatabaseFlags::default())
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

// Blame the orphan rule
fn ethrex_header_to_ret_header(header: BlockHeader) -> Header {
    Header {
        parent_hash: header.parent_hash.0.into(),
        ommers_hash: header.ommers_hash.0.into(),
        beneficiary: header.coinbase.0.into(),
        state_root: header.state_root.0.into(),
        transactions_root: header.transactions_root.0.into(),
        receipts_root: header.receipts_root.0.into(),
        withdrawals_root: header.withdrawals_root.map(|root| root.0.into()),
        logs_bloom: header.logs_bloom.0.into(),
        // FIXME: Review this later
        difficulty: Default::default(),
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        mix_hash: header.prev_randao.0.into(),
        nonce: header.nonce.to_be_bytes().into(),
        base_fee_per_gas: header.base_fee_per_gas,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root.map(|root| root.0.into()),
        requests_root: header.requests_hash.map(|hash| hash.0.into()),
        extra_data: header.extra_data.into(),
    }
}
pub fn ethrex_withdrawal_to_reth_withdrawal(original: Withdrawal) -> RethWithdrawal {
    RethWithdrawal {
        index: original.index,
        validator_index: original.validator_index,
        address: original.address.0.into(),
        amount: original.amount,
    }
}
fn reth_header_to_ethrex_header(header: Header) -> BlockHeader {
    BlockHeader {
        parent_hash: H256(header.parent_hash.0),
        ommers_hash: H256(header.ommers_hash.0),
        coinbase: H160(header.beneficiary.0 .0),
        state_root: H256(header.state_root.0),
        transactions_root: H256(header.transactions_root.0),
        receipts_root: H256(header.receipts_root.0),
        withdrawals_root: header.withdrawals_root.map(|root| H256(root.0)),
        logs_bloom: ethrex_common::Bloom(*header.logs_bloom.0),
        // FIXME: Review this later
        difficulty: Default::default(),
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data: header.extra_data.into(),
        prev_randao: H256(header.mix_hash.0),
        nonce: u64::from_be_bytes(header.nonce.0),
        base_fee_per_gas: header.base_fee_per_gas,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root.map(|root| H256(root.0)),
        requests_hash: header.requests_root.map(|hash| H256(hash.0)),
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
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().unwrap();
        Ok(tx.get::<T>(key).unwrap())
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        tx.put::<T>(key, value).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.tx_mut().unwrap();
        for (k, v) in key_values {
            txn.put::<T>(k, v).unwrap();
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
    fn get(&self, subkey: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().unwrap();
        let mut cursor = tx.cursor_dup_read::<T>().unwrap();

        let value = cursor
            .seek_by_key_subkey(
                self.fixed_key.lock().unwrap().as_ref().unwrap().clone(),
                subkey,
            )
            .unwrap();

        Ok(value)
    }

    fn put(&self, subkey: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<T>().unwrap();

        cursor
            .seek_exact(self.fixed_key.lock().unwrap().as_ref().unwrap().clone())
            .unwrap();

        cursor.append_dup(subkey.clone(), value).unwrap();

        tx.commit().unwrap();
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<T>().unwrap();

        // Position at main key once
        cursor
            .seek_exact(self.fixed_key.lock().unwrap().as_ref().unwrap().clone())
            .unwrap();

        for (subkey, value) in key_values {
            // Append each subkey+value pair
            cursor.append_dup(subkey.clone(), value).unwrap();
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
    table PayloadsTable<Key = u64, Value = Vec<u8>>;
    table TransactionLocations<Key = Vec<u8>, Value = Vec<u8>>;
    table Bodies<Key = Vec<u8>, Value = Vec<u8>>;
    table Headers<Key = Vec<u8>, Value = Vec<u8>>;
    table CanonicalBlockHashes<Key = u64, Value = Vec<u8>>;
    table BlockNumbers<Key = Vec<u8>, Value = u64>;
    table PendingBlocks<Key = Vec<u8>, Value = Vec<u8>>;
    table ChainData<Key = u8, Value = Vec<u8>>;
    table SnapState<Key = u8, Value = Vec<u8>>;
    table Payloads<Key = u64, Value = Vec<u8>>;
}

impl StoreEngine for MDBXFork {
    fn add_block_header(
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

    fn add_block_headers(
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

    fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        todo!()
        // self.env.insert_blo
    }

    fn add_blocks(&self, blocks: &[Block]) -> Result<(), StoreError> {
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

    fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_body(&self, block_number: BlockNumber) -> Result<Option<BlockBody>, StoreError> {
        todo!()
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        todo!()
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

    fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_block_number(&self, block_hash: BlockHash) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
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

    fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        todo!()
    }

    fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        todo!()
    }

    fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
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

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::EarliestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx
            .get::<ChainData>(ChainDataIndex::EarliestBlockNumber as u8)
            .unwrap()
            .map(|r| RLPDecode::decode(r.as_ref()).unwrap());
        Ok(res)
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::SafeBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(
            ChainDataIndex::LatestBlockNumber as u8,
            block_number.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx
            .get::<ChainData>(ChainDataIndex::LatestBlockNumber as u8)
            .unwrap()
            .map(|r| RLPDecode::decode(r.as_ref()).unwrap());
        Ok(res)
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        *(self.account_trie.fixed_key.lock().unwrap()) = Some(hashed_address.0.as_slice().to_vec());
        Trie::open(self.account_trie.clone(), storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        Trie::open(self.storage_trie.clone(), state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<CanonicalBlockHashes>(number, hash.encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_canonical_block_hash(
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

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<Payloads>(payload_id, PayloadBundle::from_block(block).encode_to_vec())
            .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        let tx = self.env.tx().unwrap();
        let res = tx.get::<PayloadsTable>(payload_id).unwrap();
        match res {
            Some(encoded) => Ok(Some(PayloadBundle::decode(&encoded[..]).unwrap())),
            None => Ok(None),
        }
    }

    fn update_payload(&self, payload_id: u64, payload: PayloadBundle) -> Result<(), StoreError> {
        todo!()
    }

    fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        todo!()
    }

    fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        todo!()
    }

    fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
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

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<PendingBlocks>(
            block.header.compute_block_hash().encode_to_vec(),
            block.encode_to_vec(),
        )
        .unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        todo!()
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        let mut cursor = tx.cursor_dup_write::<Receipts>().unwrap();

        let main_key = block_hash.as_bytes().to_vec();

        cursor.seek_exact(main_key.clone()).unwrap();

        for (index, receipt) in receipts.into_iter().enumerate() {
            let subkey = (index as u64).to_be_bytes().to_vec();

            let receipt_rlp = receipt.encode_to_vec();

            let value = [&subkey[..], &receipt_rlp[..]].concat();

            cursor.append_dup(main_key.clone(), value).unwrap();
        }

        tx.commit().unwrap();
        Ok(())
    }

    fn add_receipts_for_blocks(
        &self,
        receipts: std::collections::HashMap<BlockHash, Vec<Receipt>>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        todo!()
    }

    fn set_header_download_checkpoint(&self, block_hash: BlockHash) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint as u8,
            block_hash.encode_to_vec(),
        )
        .unwrap();
        Ok(())
    }

    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        let tx = self.env.tx().unwrap();
        Ok(tx
            .get::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint as u8)
            .unwrap()
            .map(|h| BlockHash::decode(h.as_ref()).unwrap()))
    }

    fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        todo!()
    }

    fn set_storage_heal_paths(
        &self,
        accounts: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_storage_heal_paths(&self) -> Result<Option<Vec<(H256, Vec<Nibbles>)>>, StoreError> {
        todo!()
    }

    fn is_synced(&self) -> Result<bool, StoreError> {
        let tx = self.env.tx().unwrap();
        let sync_status = tx
            .get::<ChainData>(ChainDataIndex::IsSynced as u8)
            .unwrap()
            .unwrap();
        Ok(RLPDecode::decode(&sync_status).unwrap())
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<ChainData>(ChainDataIndex::IsSynced as u8, status.encode_to_vec())
            .unwrap();
        Ok(())
    }

    fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        todo!()
    }

    fn clear_snap_state(&self) -> Result<(), StoreError> {
        todo!()
    }

    fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        todo!()
    }

    fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        todo!()
    }

    fn get_storage_trie_rebuild_pending(&self) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        todo!()
    }

    fn clear_snapshot(&self) -> Result<(), StoreError> {
        todo!()
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        todo!()
    }

    fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        todo!()
    }
}
