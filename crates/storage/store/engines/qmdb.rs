use crate::engines::api::StoreEngine;

use qmdb::{config::Config, test_helper::SimpleTask, AdsCore, AdsWrap};

#[derive(Debug)]
pub struct Store;

impl Store {
    pub fn new() -> Self {
        let config = Config::from_dir("ethrex.qmdb");
        AdsCore::init_dir(&config);
        let mut ads: AdsWrap<SimpleTask> = AdsWrap::new(&config);
        Self {}
    }
}

impl StoreEngine for Store {
    fn add_block_header(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_header: ethrex_core::types::BlockHeader,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_block_headers(
        &self,
        block_hashes: Vec<ethrex_core::types::BlockHash>,
        block_headers: Vec<ethrex_core::types::BlockHeader>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_header(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_body(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_body: ethrex_core::types::BlockBody,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_body(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        todo!()
    }

    fn get_block_body_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockBody>, crate::error::StoreError> {
        todo!()
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockHeader>, crate::error::StoreError> {
        todo!()
    }

    fn add_pending_block(
        &self,
        block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_pending_block(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::Block>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_number(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_number(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn add_block_total_difficulty(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        block_total_difficulty: ethrex_core::U256,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_block_total_difficulty(
        &self,
        block_hash: ethrex_core::types::BlockHash,
    ) -> Result<Option<ethrex_core::U256>, crate::error::StoreError> {
        todo!()
    }

    fn add_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
        block_number: ethrex_core::types::BlockNumber,
        block_hash: ethrex_core::types::BlockHash,
        index: ethrex_core::types::Index,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_transaction_locations(
        &self,
        locations: Vec<(
            ethrex_core::H256,
            ethrex_core::types::BlockNumber,
            ethrex_core::types::BlockHash,
            ethrex_core::types::Index,
        )>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_transaction_location(
        &self,
        transaction_hash: ethrex_core::H256,
    ) -> Result<
        Option<(
            ethrex_core::types::BlockNumber,
            ethrex_core::types::BlockHash,
            ethrex_core::types::Index,
        )>,
        crate::error::StoreError,
    > {
        todo!()
    }

    fn add_receipt(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        index: ethrex_core::types::Index,
        receipt: ethrex_core::types::Receipt,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_receipts(
        &self,
        block_hash: ethrex_core::types::BlockHash,
        receipts: Vec<ethrex_core::types::Receipt>,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipt(
        &self,
        block_number: ethrex_core::types::BlockNumber,
        index: ethrex_core::types::Index,
    ) -> Result<Option<ethrex_core::types::Receipt>, crate::error::StoreError> {
        todo!()
    }

    fn add_account_code(
        &self,
        code_hash: ethrex_core::H256,
        code: bytes::Bytes,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_account_code(
        &self,
        code_hash: ethrex_core::H256,
    ) -> Result<Option<bytes::Bytes>, crate::error::StoreError> {
        todo!()
    }

    fn get_canonical_block_hash(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<Option<ethrex_core::types::BlockHash>, crate::error::StoreError> {
        todo!()
    }

    fn set_chain_config(
        &self,
        chain_config: &ethrex_core::types::ChainConfig,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_chain_config(
        &self,
    ) -> Result<ethrex_core::types::ChainConfig, crate::error::StoreError> {
        todo!()
    }

    fn update_earliest_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_earliest_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_finalized_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_finalized_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_safe_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_safe_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_latest_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_latest_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn update_latest_total_difficulty(
        &self,
        latest_total_difficulty: ethrex_core::U256,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_latest_total_difficulty(
        &self,
    ) -> Result<Option<ethrex_core::U256>, crate::error::StoreError> {
        todo!()
    }

    fn update_pending_block_number(
        &self,
        block_number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_pending_block_number(
        &self,
    ) -> Result<Option<ethrex_core::types::BlockNumber>, crate::error::StoreError> {
        todo!()
    }

    fn open_storage_trie(
        &self,
        hashed_address: ethrex_core::H256,
        storage_root: ethrex_core::H256,
    ) -> ethrex_trie::Trie {
        todo!()
    }

    fn open_state_trie(&self, state_root: ethrex_core::H256) -> ethrex_trie::Trie {
        todo!()
    }

    fn set_canonical_block(
        &self,
        number: ethrex_core::types::BlockNumber,
        hash: ethrex_core::types::BlockHash,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn unset_canonical_block(
        &self,
        number: ethrex_core::types::BlockNumber,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn add_payload(
        &self,
        payload_id: u64,
        block: ethrex_core::types::Block,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_payload(
        &self,
        payload_id: u64,
    ) -> Result<
        Option<(
            ethrex_core::types::Block,
            ethrex_core::U256,
            ethrex_core::types::BlobsBundle,
            bool,
        )>,
        crate::error::StoreError,
    > {
        todo!()
    }

    fn update_payload(
        &self,
        payload_id: u64,
        block: ethrex_core::types::Block,
        block_value: ethrex_core::U256,
        blobs_bundle: ethrex_core::types::BlobsBundle,
        completed: bool,
    ) -> Result<(), crate::error::StoreError> {
        todo!()
    }

    fn get_receipts_for_block(
        &self,
        block_hash: &ethrex_core::types::BlockHash,
    ) -> Result<Vec<ethrex_core::types::Receipt>, crate::error::StoreError> {
        todo!()
    }
}
