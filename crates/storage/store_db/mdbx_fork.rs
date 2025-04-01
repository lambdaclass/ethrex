// Storage implementation using reth's fork of libmdbx
// to compare against our own.
use std::sync::{Arc, Mutex};

use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
    BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, Rlp, TransactionHashRLP, TupleRLP,
};
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::utils::{ChainDataIndex, SnapStateIndex};
use alloy_consensus::Header;
use anyhow::Result;
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::types::{
    payload::PayloadBundle, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
    ChainConfig, Index, Receipt, Transaction,
};
use ethrex_common::{Bloom, H160};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, Trie};
use reth_db::mdbx::{init_db, DatabaseArguments, DatabaseEnv};
use reth_db::tables;
use reth_db::{
    transaction::{DbTx, DbTxMut},
    Database,
};

#[derive(Debug)]
pub struct MDBXFork {
    env: DatabaseEnv,
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
    }

    fn add_blocks(&self, blocks: &[Block]) -> Result<(), StoreError> {
        todo!()
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
        todo!()
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
        todo!()
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
        todo!()
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        todo!()
    }

    fn update_earliest_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
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
        todo!()
    }

    fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn update_pending_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        todo!()
    }

    fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        todo!()
    }

    fn open_storage_trie(&self, hashed_address: H256, storage_root: H256) -> Trie {
        todo!()
    }

    fn open_state_trie(&self, state_root: H256) -> Trie {
        todo!()
    }

    fn set_canonical_block(&self, number: BlockNumber, hash: BlockHash) -> Result<(), StoreError> {
        todo!()
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
        todo!()
    }

    fn update_sync_status(&self, status: bool) -> Result<(), StoreError> {
        todo!()
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
