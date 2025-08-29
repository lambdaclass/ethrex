mod payload_builder;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_blockchain::{
    Blockchain,
    error::ChainError,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
    validate_block,
};
use ethrex_common::Address;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::BlockExecutionResult;
use keccak_hash::H256;
use payload_builder::build_payload;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{debug, error, info};

use crate::{
    BlockProducerConfig, SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
};

use super::errors::BlockProducerError;

use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{metrics_blocks::METRICS_BLOCKS, metrics_transactions::METRICS_TX};

#[derive(Clone)]
pub enum InMessage {
    Produce,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct BlockProducer {
    store: Store,
    blockchain: Arc<Blockchain>,
    sequencer_state: SequencerState,
    block_time_ms: u64,
    coinbase_address: Address,
    elasticity_multiplier: u64,
    rollup_store: StoreRollup,
    max_gas_limit: u64,
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
            elasticity_multiplier,
            max_gas_limit,
        } = config;
        Self {
            store,
            blockchain,
            sequencer_state,
            block_time_ms: *block_time_ms,
            coinbase_address: *coinbase_address,
            elasticity_multiplier: *elasticity_multiplier,
            rollup_store,
            max_gas_limit: *max_gas_limit,
        }
    }

    pub async fn spawn(
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<(), BlockProducerError> {
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
        Ok(())
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
        };
        let payload = create_payload(&args, &self.store)?;

        // Blockchain builds the payload from mempool txs and executes them
        let payload_build_result = build_payload(
            self.blockchain.clone(),
            payload,
            &self.store,
            &self.rollup_store,
            self.max_gas_limit,
        )
        .await?;
        info!(
            "Built payload for new block {}",
            payload_build_result.payload.header.number
        );

        // Blockchain stores block
        let block = payload_build_result.payload;
        let chain_config = self.store.get_chain_config()?;
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
            .apply_account_updates_batch(block.header.parent_hash, &account_updates)
            .await?
            .ok_or(ChainError::ParentStateNotFound)?;

        self.blockchain
            .store_block(&block, account_updates_list, execution_result)
            .await?;
        info!("Stored new block {:x}", block.hash());
        // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

        self.rollup_store
            .store_account_updates_by_block_number(block.header.number, account_updates)
            .await?;

        // Make the new head be part of the canonical chain
        apply_fork_choice(&self.store, block.hash(), block.hash(), block.hash()).await?;

        metrics!(
            let _ = METRICS_BLOCKS
            .set_block_number(block.header.number)
            .inspect_err(|e| {
                tracing::error!("Failed to set metric: block_number {}", e.to_string())
            });
            #[allow(clippy::as_conversions)]
            let tps = block.body.transactions.len() as f64 / (self.block_time_ms as f64 / 1000_f64);
            METRICS_TX.set_transactions_per_second(tps);
        );

        Ok(())
    }
}

impl GenServer for BlockProducer {
    type CallMsg = Unused;
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
}
