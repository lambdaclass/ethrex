use ethrex_trie::InMemoryTrieDB;
use reth_provider::providers::StaticFileProvider;
use std::cell::{Cell, LazyCell, OnceCell};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
// Storage implementation using reth's fork of libmdbx
// to compare against our own.
use reth_provider::{DatabaseProvider, StaticFileAccess};
use std::sync::{Arc, LazyLock, Mutex};

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
    BlockBody as RethBlockBody, SealedBlock, SealedBlockWithSenders, TransactionSigned, Withdrawals,
};
use reth_primitives_traits::SealedHeader;
use reth_provider::BlockWriter;
use reth_storage_api::DBProvider;
use std::collections::HashMap;

#[derive(Debug)]
pub struct MDBXFork {
    env: DatabaseEnv,
}

pub static SYNC_STATUS: AtomicBool = AtomicBool::new(false);
pub static LATEST_BLOCK_NUMBER: AtomicU64 = AtomicU64::new(0);
pub static EARLIEST_BLOCK_NUMBER: AtomicU64 = AtomicU64::new(0);
lazy_static::lazy_static! {
    pub static ref CHAIN_CONFIG: Arc<Mutex<Option<ChainConfig>>> = Default::default() ;
    pub static ref STORAGE_TRIE: InMemoryTrieDB = Default::default();
    pub static ref STATE_TRIE: InMemoryTrieDB = Default::default();
}

impl MDBXFork {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        let client_version = Default::default();
        let db_args = DatabaseArguments::new(client_version);
        let env = init_db(path, db_args).expect("Failed to initialize MDBX Fork");
        Ok(Self { env })
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
use reth_libmdbx::Environment;

pub struct MDBXTrieDB<T: RethTable> {
    db: DatabaseEnv,
    phantom: PhantomData<T>,
}
impl<T> MDBXTrieDB<T>
where
    T: RethTable,
{
    pub fn new(db: DatabaseEnv) -> Self {
        Self {
            db,
            phantom: PhantomData,
        }
    }
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
        let block_number = block_header.number;
        tx.put::<tables::HeaderNumbers>(block_hash.0.into(), block_number)
            .unwrap();
        tx.put::<tables::Headers>(block_number, ethrex_header_to_ret_header(block_header))
            .unwrap();
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
            let block_number = header.number;
            tx.put::<tables::HeaderNumbers>(hash.0.into(), block_number)
                .unwrap();
            tx.put::<tables::Headers>(block_number, ethrex_header_to_ret_header(header))
                .unwrap();
        }
        Ok(())
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let tx = self.env.tx().expect("Could not start tx for block headers");
        let header = tx.get::<tables::Headers>(block_number).unwrap();
        Ok(header.map(|header| reth_header_to_ethrex_header(header)))
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
        let pre_processed: Vec<_> = blocks
            .iter()
            .map(|Block { header, body, .. }| {
                let hash = header.compute_block_hash();
                let header =
                    SealedHeader::new(ethrex_header_to_ret_header(header.clone()), hash.0.into());
                let ommers = body
                    .ommers
                    .iter()
                    .map(|h| ethrex_header_to_ret_header(h.clone()))
                    .collect();
                let ws: Option<Withdrawals> = body.withdrawals.as_ref().map(|ws| {
                    Withdrawals::new(
                        ws.iter()
                            .map(|w| ethrex_withdrawal_to_reth_withdrawal(w.clone()))
                            .collect(),
                    )
                });
                let transactions: Vec<_> = body
                    .transactions
                    .iter()
                    // FIXME: Properly transform the tx type
                    .map(|_tx| TransactionSigned::default())
                    .collect();

                let body = RethBlockBody {
                    transactions: transactions.clone(),
                    ommers,
                    withdrawals: ws,
                    requests: None,
                };
                let senders = transactions
                    .iter()
                    .map(|_tx_| {
                        // FIXME: Properly transform the tx type
                        let tx = TransactionSigned::default();
                        tx.recover_signer().unwrap()
                    })
                    .collect();
                let block = SealedBlock::new(header, body);
                SealedBlockWithSenders::new(block, senders).unwrap()
            })
            .collect();
        let tx = self.env.tx_mut().unwrap();
        let provider = StaticFileProvider::read_write("/tmp/provider").unwrap();
        // let db_provider = DatabaseProvider
        let spec: ChainSpec = Default::default();
        let provider = DatabaseProvider::new_rw(tx, Arc::new(spec), provider, Default::default());
        for block in pre_processed {
            dbg!(block.hash());
            provider.insert_block(block).unwrap();
        }
        provider.commit().unwrap();
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
        dbg!(&block_hash);
        let tx = self.env.tx().unwrap();
        let block_number = tx
            .get::<tables::HeaderNumbers>(block_hash.0.into())
            .unwrap()
            .unwrap();
        let header = tx.get::<tables::Headers>(block_number).unwrap().unwrap();
        Ok(Some(reth_header_to_ethrex_header(header)))
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
        Ok(())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        todo!()
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
        let mut config = CHAIN_CONFIG.lock().unwrap();
        *config = Some(chain_config.clone());
        Ok(())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        Ok(CHAIN_CONFIG.lock().unwrap().unwrap().clone())
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        EARLIEST_BLOCK_NUMBER.swap(block_number, std::sync::atomic::Ordering::Acquire);
        Ok(())
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(Some(
            EARLIEST_BLOCK_NUMBER.load(std::sync::atomic::Ordering::Acquire),
        ))
    }

    fn update_finalized_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_latest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        LATEST_BLOCK_NUMBER.swap(block_number, std::sync::atomic::Ordering::Acquire);
        Ok(())
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        Ok(Some(
            LATEST_BLOCK_NUMBER.load(std::sync::atomic::Ordering::Acquire),
        ))
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        let db = Box::new(STORAGE_TRIE.clone());
        Trie::open(db, storage_root)
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        let db = Box::new(STATE_TRIE.clone());
        Trie::open(db, state_root)
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        let tx = self.env.tx_mut().unwrap();
        tx.put::<tables::CanonicalHeaders>(number, hash.0.into())
            .unwrap();
        Ok(())
    }

    fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        todo!()
    }

    fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        todo!()
    }

    fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        todo!()
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
        todo!()
    }

    fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        todo!()
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
        todo!()
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
        todo!()
    }

    fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        todo!()
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
        Ok(SYNC_STATUS.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        SYNC_STATUS.store(status, std::sync::atomic::Ordering::Relaxed);
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
