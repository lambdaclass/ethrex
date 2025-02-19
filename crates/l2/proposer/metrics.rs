use crate::{
    proposer::errors::MetricsGathererError,
    utils::config::{
        committer::CommitterConfig, errors::ConfigError, eth::EthConfig,
        l1_watcher::L1WatcherConfig,
    },
};
use ethereum_types::Address;
use ethrex_metrics::metrics_l2::{MetricsL2BlockType, METRICS_L2};
use ethrex_rpc::clients::eth::{errors::EthClientError, EthClient};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error};

pub async fn start_metrics_gatherer() -> Result<(), ConfigError> {
    let eth_config = EthConfig::from_env()?;
    // Just for the CommonBridge address
    let watcher_config = L1WatcherConfig::from_env()?;
    // Just for the OnChainProposer Address
    let committer_config = CommitterConfig::from_env()?;
    let mut metrics_gatherer =
        MetricsGatherer::new_from_config(watcher_config, committer_config, eth_config).await?;
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
        watcher_config: L1WatcherConfig,
        committer_config: CommitterConfig,
        eth_config: EthConfig,
    ) -> Result<Self, EthClientError> {
        let eth_client = EthClient::new(&eth_config.rpc_url);
        //let l2_client = EthClient::new("http://localhost:1729");
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

    async fn main_logic(&mut self) -> Result<(), MetricsGathererError> {
        loop {
            let last_fetched_l1_block =
                EthClient::get_last_fetched_l1_block(&self.eth_client, self.common_bridge_address)
                    .await?;

            let last_committed_block = EthClient::get_last_committed_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            let last_verified_block = EthClient::get_last_verified_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            METRICS_L2.set_block_type_and_block_number(
                MetricsL2BlockType::LastCommittedBlock,
                last_committed_block,
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
