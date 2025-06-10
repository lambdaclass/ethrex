mod payload_builder;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_blockchain::{
    fork_choice::apply_fork_choice,
    payload::{create_payload, BuildPayloadArgs},
    validate_block, Blockchain,
};
use ethrex_common::Address;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::BlockExecutionResult;
use keccak_hash::H256;
use payload_builder::build_payload;
use tokio::time::sleep;
use tracing::{debug, error, info};

use crate::{BlockProducerConfig, SequencerConfig};

use super::errors::{BlockProducerError, SequencerError};

use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{metrics_blocks::METRICS_BLOCKS, metrics_transactions::METRICS_TX};

pub struct BlockProducer {
    block_time_ms: u64,
    coinbase_address: Address,
    elasticity_multiplier: u64,
}

pub async fn start_block_producer(
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let proposer = BlockProducer::new_from_config(&cfg.block_producer);
    proposer
        .run(store.clone(), rollup_store.clone(), blockchain)
        .await;
    Ok(())
}

impl BlockProducer {
    pub fn new_from_config(config: &BlockProducerConfig) -> Self {
        let BlockProducerConfig {
            block_time_ms,
            coinbase_address,
            elasticity_multiplier,
        } = config;
        Self {
            block_time_ms: *block_time_ms,
            coinbase_address: *coinbase_address,
            elasticity_multiplier: *elasticity_multiplier,
        }
    }

    pub async fn run(&self, store: Store, rollup_store: StoreRollup, blockchain: Arc<Blockchain>) {
        loop {
            let _ = self
                .main_logic(store.clone(), rollup_store.clone(), blockchain.clone())
                .await
                .inspect_err(|e| error!("Block Producer Error: {e}"));

            sleep(Duration::from_millis(self.block_time_ms)).await;
        }
    }

    pub async fn main_logic(
        &self,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
    ) -> Result<(), BlockProducerError> {
        let version = 3;
        let head_header = {
            let current_block_number = store.get_latest_block_number().await?;
            store
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
        let payload = create_payload(&args, &store)?;

        // Blockchain builds the payload from mempool txs and executes them
        let payload_build_result = build_payload(blockchain.clone(), payload, &store).await?;
        info!(
            "Built payload for new block {}",
            payload_build_result.payload.header.number
        );

        // Blockchain stores block
        let block = payload_build_result.payload;
        let chain_config = store.get_chain_config()?;
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

        blockchain
            .store_block(&block, execution_result.clone(), &account_updates)
            .await?;
        info!("Stored new block {:x}", block.hash());
        // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

        // Store execution result
        rollup_store
            .store_account_updates_by_block_number(block.header.number, account_updates)
            .await?;

        // Make the new head be part of the canonical chain
        apply_fork_choice(&store, block.hash(), block.hash(), block.hash()).await?;

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
