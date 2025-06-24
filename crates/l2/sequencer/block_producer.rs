mod payload_builder;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_blockchain::{
    Blockchain,
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
use spawned_concurrency::{CallResponse, CastResponse, GenServer, GenServerInMsg, send_after};
use spawned_rt::mpsc::Sender;
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
pub struct BlockProducerState {
    store: Store,
    blockchain: Arc<Blockchain>,
    sequencer_state: SequencerState,
    block_time_ms: u64,
    coinbase_address: Address,
    elasticity_multiplier: u64,
    rollup_store: StoreRollup,
}

impl BlockProducerState {
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
        } = config;
        Self {
            store,
            blockchain,
            sequencer_state,
            block_time_ms: *block_time_ms,
            coinbase_address: *coinbase_address,
            elasticity_multiplier: *elasticity_multiplier,
            rollup_store,
        }
    }
}

#[derive(Clone)]
pub enum InMessage {
    Produce,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct BlockProducer;

impl BlockProducer {
    pub async fn spawn(
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<(), BlockProducerError> {
        let state = BlockProducerState::new(
            &cfg.block_producer,
            store,
            rollup_store,
            blockchain,
            sequencer_state,
        );
        let mut block_producer = BlockProducer::start(state);
        block_producer
            .cast(InMessage::Produce)
            .await
            .map_err(BlockProducerError::GenServerError)?;
        Ok(())
    }
}

impl GenServer for BlockProducer {
    type InMsg = InMessage;
    type OutMsg = OutMessage;
    type State = BlockProducerState;

    type Error = BlockProducerError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_call(
        &mut self,
        _message: Self::InMsg,
        _tx: &Sender<GenServerInMsg<Self>>,
        _state: &mut Self::State,
    ) -> CallResponse<Self::OutMsg> {
        CallResponse::Reply(OutMessage::Done)
    }

    async fn handle_cast(
        &mut self,
        _message: Self::InMsg,
        tx: &Sender<GenServerInMsg<Self>>,
        state: &mut Self::State,
    ) -> CastResponse {
        // Right now we only have the Produce message, so we ignore the message
        if let SequencerStatus::Sequencing = state.sequencer_state.status().await {
            let _ = produce_block(state)
                .await
                .inspect_err(|e| error!("Block Producer Error: {e}"));
        }
        send_after(
            Duration::from_millis(state.block_time_ms),
            tx.clone(),
            Self::InMsg::Produce,
        );
        CastResponse::NoReply
    }
}

pub async fn produce_block(state: &BlockProducerState) -> Result<(), BlockProducerError> {
    let version = 3;
    let head_header = {
        let current_block_number = state.store.get_latest_block_number().await?;
        state
            .store
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
        fee_recipient: state.coinbase_address,
        random: H256::zero(),
        withdrawals: Default::default(),
        beacon_root: Some(head_beacon_block_root),
        version,
        elasticity_multiplier: state.elasticity_multiplier,
    };
    let payload = create_payload(&args, &state.store)?;

    // Blockchain builds the payload from mempool txs and executes them
    let payload_build_result =
        build_payload(state.blockchain.clone(), payload, &state.store).await?;
    info!(
        "Built payload for new block {}",
        payload_build_result.payload.header.number
    );

    // Blockchain stores block
    let block = payload_build_result.payload;
    let chain_config = state.store.get_chain_config()?;
    validate_block(
        &block,
        &head_header,
        &chain_config,
        state.elasticity_multiplier,
    )?;

    let account_updates = payload_build_result.account_updates;

    let execution_result = BlockExecutionResult {
        receipts: payload_build_result.receipts,
        requests: Vec::new(),
    };

    state
        .blockchain
        .store_block(&block, execution_result, &account_updates)
        .await?;
    info!("Stored new block {:x}", block.hash());
    // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

    state
        .rollup_store
        .store_account_updates_by_block_number(block.header.number, account_updates)
        .await?;

    // Make the new head be part of the canonical chain
    apply_fork_choice(&state.store, block.hash(), block.hash(), block.hash()).await?;

    metrics!(
        let _ = METRICS_BLOCKS
        .set_block_number(block.header.number)
        .inspect_err(|e| {
            tracing::error!("Failed to set metric: block_number {}", e.to_string())
        });
        #[allow(clippy::as_conversions)]
        let tps = block.body.transactions.len() as f64 / (state.block_time_ms as f64 / 1000_f64);
        METRICS_TX.set_transactions_per_second(tps);
    );

    Ok(())
}
