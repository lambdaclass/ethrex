mod payload_builder;
use std::{
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainType,
    error::ChainError,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
    validate_block,
};
use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_levm::hooks::l2_hook::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::BlockExecutionResult;
pub use payload_builder::build_payload;
use serde::Serialize;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_after,
};
use tracing::{debug, error, info, warn};

use crate::{
    BlockProducerConfig, SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::l1_committer::{self, L1Committer},
};

use super::errors::BlockProducerError;

use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{metrics_blocks::METRICS_BLOCKS, metrics_transactions::METRICS_TX};

// 0x85a190caa61692b36b63a55e069330d18ab9af179fed7a25c16a4262bc63b7d2
const DEPOSIT_PROCESSED_SIGNATURE: H256 = H256([
    0x85, 0xa1, 0x90, 0xca, 0xa6, 0x16, 0x92, 0xb3, 0x6b, 0x63, 0xa5, 0x5e, 0x06, 0x93, 0x30, 0xd1,
    0x8a, 0xb9, 0xaf, 0x17, 0x9f, 0xed, 0x7a, 0x25, 0xc1, 0x6a, 0x42, 0x62, 0xbc, 0x63, 0xb7, 0xd2,
]);

// 0xf5353a2477e10b23280de25ca6cea55c17bb48000d8807ee631e514080e7fb4e
const ERC20_DEPOSIT_PROCESSED_SIGNATURE: H256 = H256([
    0xf5, 0x35, 0x3a, 0x24, 0x77, 0xe1, 0x0b, 0x23, 0x28, 0x0d, 0xe2, 0x5c, 0xa6, 0xce, 0xa5, 0x5c,
    0x17, 0xbb, 0x48, 0x00, 0x0d, 0x88, 0x07, 0xee, 0x63, 0x1e, 0x51, 0x40, 0x80, 0xe7, 0xfb, 0x4e,
]);

// 0xbb2689ff876f7ef453cf8865dde5ab10349d222e2e1383c5152fbdb083f02da2
const WITHDRAWAL_INITIATED_SIGNATURE: H256 = H256([
    0xbb, 0x26, 0x89, 0xff, 0x87, 0x6f, 0x7e, 0xf4, 0x53, 0xcf, 0x88, 0x65, 0xdd, 0xe5, 0xab, 0x10,
    0x34, 0x9d, 0x22, 0x2e, 0x2e, 0x13, 0x83, 0xc5, 0x15, 0x2f, 0xbd, 0xb0, 0x83, 0xf0, 0x2d, 0xa2,
]);

// 0x54538b93c6e9b3f518076db2d896122f653fac2bb32fa0b6bc75097b9f332e75
const ERC20_WITHDRAWAL_SIGNATURE: H256 = H256([
    0x54, 0x53, 0x8b, 0x93, 0xc6, 0xe9, 0xb3, 0xf5, 0x18, 0x07, 0x6d, 0xb2, 0xd8, 0x96, 0x12, 0x2f,
    0x65, 0x3f, 0xac, 0x2b, 0xb3, 0x2f, 0xa0, 0xb6, 0xbc, 0x75, 0x09, 0x7b, 0x9f, 0x33, 0x2e, 0x75,
]);

#[derive(Clone)]
pub enum CallMessage {
    Health,
}

#[derive(Clone)]
pub enum InMessage {
    Produce,
}

#[derive(Clone)]
pub enum OutMessage {
    Done,
    Health(BlockProducerHealth),
}

pub struct BlockProducer {
    store: Store,
    blockchain: Arc<Blockchain>,
    sequencer_state: SequencerState,
    block_time_ms: u64,
    coinbase_address: Address,
    elasticity_multiplier: u64,
    rollup_store: StoreRollup,
    // Needed to ensure privileged tx nonces are sequential
    last_privileged_nonce: Option<u64>,
    block_gas_limit: u64,
    committer: GenServerHandle<L1Committer>,
}

#[derive(Clone, Serialize)]
pub struct BlockProducerHealth {
    sequencer_state: String,
    block_time_ms: u64,
    coinbase_address: Address,
    elasticity_multiplier: u64,
}

impl BlockProducer {
    pub fn new(
        config: &BlockProducerConfig,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        sequencer_state: SequencerState,
        committer: GenServerHandle<L1Committer>,
    ) -> Self {
        let BlockProducerConfig {
            block_time_ms,
            coinbase_address,
            base_fee_vault_address,
            operator_fee_vault_address,
            elasticity_multiplier,
            block_gas_limit,
        } = config;

        if base_fee_vault_address.is_some_and(|base_fee_vault| base_fee_vault == *coinbase_address)
        {
            warn!(
                "The coinbase address and base fee vault address are the same. Coinbase balance behavior will be affected.",
            );
        }
        if operator_fee_vault_address
            .is_some_and(|operator_fee_vault| operator_fee_vault == *coinbase_address)
        {
            warn!(
                "The coinbase address and operator fee vault address are the same. Coinbase balance behavior will be affected.",
            );
        }

        Self {
            store,
            blockchain,
            sequencer_state,
            block_time_ms: *block_time_ms,
            coinbase_address: *coinbase_address,
            elasticity_multiplier: *elasticity_multiplier,
            rollup_store,
            // FIXME: Initialize properly to the last privileged nonce in the chain
            last_privileged_nonce: None,
            block_gas_limit: *block_gas_limit,
            committer,
        }
    }

    pub async fn spawn(
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        committer: GenServerHandle<L1Committer>,
    ) -> Result<GenServerHandle<BlockProducer>, BlockProducerError> {
        let mut block_producer = Self::new(
            &cfg.block_producer,
            store,
            rollup_store,
            blockchain,
            sequencer_state,
            committer,
        )
        .start_blocking();
        block_producer
            .cast(InMessage::Produce)
            .await
            .map_err(BlockProducerError::InternalError)?;
        Ok(block_producer)
    }

    pub async fn produce_block(&mut self) -> Result<(), BlockProducerError> {
        let version = 3;
        let head_header = {
            let current_block_number = self.store.get_latest_block_number().await?;
            self.store
                .get_block_header(current_block_number)?
                .ok_or(BlockProducerError::StorageDataIsNone)?
        };
        let head_hash = head_header.hash();
        let head_beacon_block_root = H256::zero();

        // The proposer leverages the execution payload framework used for the engine API,
        // but avoids calling the API methods and unnecesary re-execution.

        info!("Producing block");
        debug!("Head block hash: {head_hash:#x}");

        // Proposer creates a new payload
        let args = BuildPayloadArgs {
            parent: head_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.coinbase_address,
            random: H256::zero(),
            withdrawals: Default::default(),
            beacon_root: Some(head_beacon_block_root),
            version,
            elasticity_multiplier: self.elasticity_multiplier,
            gas_ceil: self.block_gas_limit,
        };
        let payload = create_payload(&args, &self.store, Bytes::new())?;

        // Blockchain builds the payload from mempool txs and executes them
        let payload_build_result = build_payload(
            self.blockchain.clone(),
            payload,
            &self.store,
            &mut self.last_privileged_nonce,
            self.block_gas_limit,
        )
        .await?;
        let force_commitment = payload_build_result.receipts.iter().any(|receipt| {
            receipt.logs.iter().any(|log| {
                log.address == COMMON_BRIDGE_L2_ADDRESS
                    && (log.topics[0] == DEPOSIT_PROCESSED_SIGNATURE
                        || log.topics[0] == ERC20_DEPOSIT_PROCESSED_SIGNATURE
                        || log.topics[0] == WITHDRAWAL_INITIATED_SIGNATURE
                        || log.topics[0] == ERC20_WITHDRAWAL_SIGNATURE)
            })
        });
        info!(
            force_commitment,
            "Built payload for new block {}", payload_build_result.payload.header.number
        );

        // Blockchain stores block
        let block = payload_build_result.payload;
        let chain_config = self.store.get_chain_config();
        validate_block(
            &block,
            &head_header,
            &chain_config,
            self.elasticity_multiplier,
        )?;

        let account_updates = payload_build_result.account_updates;

        let execution_result = BlockExecutionResult {
            receipts: payload_build_result.receipts,
            requests: Vec::new(),
        };

        let account_updates_list = self
            .store
            .apply_account_updates_batch(block.header.parent_hash, &account_updates)?
            .ok_or(ChainError::ParentStateNotFound)?;

        let transactions_count = block.body.transactions.len();
        let block_number = block.header.number;
        let block_hash = block.hash();
        self.store_fee_config_by_block(block.header.number).await?;
        self.blockchain
            .store_block(block.clone(), account_updates_list, execution_result)?;
        info!(
            "Stored new block {:x}, transaction_count {}",
            block_hash, transactions_count
        );
        // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

        self.rollup_store
            .store_account_updates_by_block_number(block_number, account_updates)
            .await?;

        // Make the new head be part of the canonical chain
        apply_fork_choice(&self.store, block_hash, block_hash, block_hash).await?;

        metrics!(
            METRICS_BLOCKS.set_block_number(block_number);
            #[allow(clippy::as_conversions)]
            let tps = transactions_count as f64 / (self.block_time_ms as f64 / 1000_f64);
            METRICS_TX.set_transactions_per_second(tps);
        );

        println!(
            "[L2 Builder] Block {} ({:#x}) {{",
            block.header.number,
            block.hash(),
        );
        println!(
            "{}",
            block
                .body
                .transactions
                .iter()
                .map(|tx| format!("\t{:#x}", tx.hash()))
                .collect::<Vec<String>>()
                .join("\n")
        );
        println!("}}");

        if force_commitment {
            self.committer
                .cast(l1_committer::InMessage::ForceCommit)
                .await?;
        }

        Ok(())
    }
    async fn store_fee_config_by_block(&self, block_number: u64) -> Result<(), BlockProducerError> {
        let BlockchainType::L2(l2_config) = &self.blockchain.options.r#type else {
            error!("Invalid blockchain type. Expected L2.");
            return Err(BlockProducerError::Custom("Invalid blockchain type".into()));
        };

        let fee_config = *l2_config
            .fee_config
            .read()
            .map_err(|_| BlockProducerError::Custom("Fee config lock was poisoned".to_string()))?;

        self.rollup_store
            .store_fee_config_by_block(block_number, fee_config)
            .await?;
        Ok(())
    }
}

impl GenServer for BlockProducer {
    type CallMsg = CallMessage;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = BlockProducerError;

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        // Right now we only have the Produce message, so we ignore the message
        if let SequencerStatus::Sequencing = self.sequencer_state.status().await {
            let _ = self
                .produce_block()
                .await
                .inspect_err(|e| error!("Block Producer Error: {e}"));
        }
        send_after(
            Duration::from_millis(self.block_time_ms),
            handle.clone(),
            Self::CastMsg::Produce,
        );
        CastResponse::NoReply
    }

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::Health => CallResponse::Reply(OutMessage::Health(BlockProducerHealth {
                sequencer_state: format!("{:?}", self.sequencer_state.status().await),
                block_time_ms: self.block_time_ms,
                coinbase_address: self.coinbase_address,
                elasticity_multiplier: self.elasticity_multiplier,
            })),
        }
    }
}
