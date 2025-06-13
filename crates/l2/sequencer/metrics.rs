use crate::{
    CommitterConfig, EthConfig, SequencerConfig,
    sequencer::errors::{MetricsGathererError, SequencerError},
};
use ::ethrex_storage_rollup::StoreRollup;
use ethereum_types::Address;
use ethrex_metrics::metrics_l2::{METRICS_L2, MetricsL2BlockType, MetricsL2OperationType};
use ethrex_metrics::metrics_transactions::METRICS_TX;
use ethrex_rpc::clients::eth::EthClient;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error};

pub async fn start_metrics_gatherer(
    cfg: SequencerConfig,
    rollup_store: StoreRollup,
    l2_url: String,
) -> Result<(), SequencerError> {
    let mut metrics_gatherer =
        MetricsGatherer::new_from_config(rollup_store, &cfg.l1_committer, &cfg.eth, l2_url).await?;
    metrics_gatherer.run().await;
    Ok(())
}

pub struct MetricsGatherer {
    l1_eth_client: EthClient,
    l2_eth_client: EthClient,
    on_chain_proposer_address: Address,
    check_interval: Duration,
    rollup_store: StoreRollup,
}

impl MetricsGatherer {
    pub async fn new_from_config(
        rollup_store: StoreRollup,
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
        l2_url: String,
    ) -> Result<Self, MetricsGathererError> {
        let l1_eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let l2_eth_client = EthClient::new(&l2_url)?;
        Ok(Self {
            l1_eth_client,
            l2_eth_client,
            rollup_store,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            check_interval: Duration::from_millis(1000),
        })
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(err) = self.main_logic().await {
                error!("Metrics Gatherer Error: {}", err);
            }

            sleep(self.check_interval).await;
        }
    }

    async fn main_logic(&mut self) -> Result<(), MetricsGathererError> {
        loop {
            let last_committed_batch = self
                .l1_eth_client
                .get_last_committed_batch(self.on_chain_proposer_address)
                .await?;

            let last_verified_batch = self
                .l1_eth_client
                .get_last_verified_batch(self.on_chain_proposer_address)
                .await?;

            let l1_gas_price = self.l1_eth_client.get_gas_price().await?;
            let l2_gas_price = self.l2_eth_client.get_gas_price().await?;

            if let Ok(Some(last_verified_batch_blocks)) = self
                .rollup_store
                .get_block_numbers_by_batch(last_verified_batch)
                .await
            {
                if let Some(last_block) = last_verified_batch_blocks.last() {
                    METRICS_L2.set_block_type_and_block_number(
                        MetricsL2BlockType::LastVerifiedBlock,
                        *last_block,
                    )?;
                }
            }

            if let Ok(operations_metrics) = self.rollup_store.get_operations_count().await {
                let (transactions, deposits, withdrawals) = (
                    operations_metrics[0],
                    operations_metrics[1],
                    operations_metrics[2],
                );
                METRICS_L2.set_operation_by_type(MetricsL2OperationType::Deposits, deposits)?;
                METRICS_L2
                    .set_operation_by_type(MetricsL2OperationType::Withdrawals, withdrawals)?;
                METRICS_TX.set_tx_count(transactions)?;
            }

            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastCommittedBatch,
                last_committed_batch,
            )?;
            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastVerifiedBatch,
                last_verified_batch,
            )?;
            METRICS_L2.set_l1_gas_price(
                l1_gas_price
                    .try_into()
                    .map_err(|e: &str| MetricsGathererError::TryInto(e.to_string()))?,
            );
            METRICS_L2.set_l2_gas_price(
                l2_gas_price
                    .try_into()
                    .map_err(|e: &str| MetricsGathererError::TryInto(e.to_string()))?,
            );

            debug!("L2 Metrics Gathered");
            sleep(self.check_interval).await;
        }
    }
}
