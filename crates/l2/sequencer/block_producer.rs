mod payload_builder;
use std::{
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
};

use super::errors::BlockProducerError;

use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{metrics_blocks::METRICS_BLOCKS, metrics_transactions::METRICS_TX};

#[derive(Clone)]
pub enum CallMessage {
    Health,
}

#[derive(Clone)]
pub enum InMessage {
    Produce,
    Abort,
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
    // Needed to ensure privileged tx nonces are sequential per source chain
    privileged_nonces: std::collections::HashMap<u64, Option<u64>>,
    block_gas_limit: u64,
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
            privileged_nonces: std::collections::HashMap::new(),
            block_gas_limit: *block_gas_limit,
        }
    }

    pub async fn spawn(
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<GenServerHandle<BlockProducer>, BlockProducerError> {
        let mut block_producer = Self::new(
            &cfg.block_producer,
            store,
            rollup_store,
            blockchain,
            sequencer_state,
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
            &mut self.privileged_nonces,
            self.block_gas_limit,
        )
        .await?;
        info!(
            "Built payload for new block {}",
            payload_build_result.payload.header.number
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
            .store_block(block, account_updates_list, execution_result)?;
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
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            InMessage::Produce => {
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
            InMessage::Abort => {
                // start_blocking keeps this GenServer alive even if the JoinSet aborts the task.
                // Returning CastResponse::Stop is how the blocking runner actually shuts down.
                CastResponse::Stop
            }
        }
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
