use crate::{
    sequencer::errors::{MetricsGathererError, SequencerError},
    CommitterConfig, EthConfig, L1WatcherConfig, SequencerConfig,
};
use ethereum_types::Address;
use ethrex_metrics::metrics_l2::{MetricsL2BlockType, METRICS_L2};
use ethrex_rpc::clients::eth::EthClient;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error};

pub async fn start_metrics_gatherer(cfg: SequencerConfig) -> Result<(), SequencerError> {
    let mut metrics_gatherer =
        MetricsGatherer::new_from_config(&cfg.l1_watcher, &cfg.l1_committer, &cfg.eth).await?;
    metrics_gatherer.run().await;
    Ok(())
}

pub struct MetricsGatherer {
    eth_client: EthClient,
    common_bridge_address: Address,
    on_chain_proposer_address: Address,
    check_interval: Duration,
}

impl MetricsGatherer {
    pub async fn new_from_config(
        watcher_config: &L1WatcherConfig,
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
    ) -> Result<Self, MetricsGathererError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        Ok(Self {
            eth_client,
            common_bridge_address: watcher_config.bridge_address,
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

    // TODO: update metrics to work with batches.
    async fn main_logic(&mut self) -> Result<(), MetricsGathererError> {
        loop {
            let last_fetched_l1_block = self
                .eth_client
                .get_last_fetched_l1_block(self.common_bridge_address)
                .await?;

            let last_committed_batch = self
                .eth_client
                .get_last_committed_batch(self.on_chain_proposer_address)
                .await?;

            let last_verified_block = self
                .eth_client
                .get_last_verified_batch(self.on_chain_proposer_address)
                .await?;

            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastCommittedBlock,
                last_committed_batch,
            )?;
            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastVerifiedBlock,
                last_verified_block,
            )?;
            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastFetchedL1Block,
                last_fetched_l1_block,
            )?;

            debug!("L2 Metrics Gathered");
            sleep(self.check_interval).await;
        }
    }
}
