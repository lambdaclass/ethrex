use crate::{CommitterConfig, EthConfig, SequencerConfig, sequencer::errors::MetricsGathererError};
use ::ethrex_storage_rollup::StoreRollup;
use ethereum_types::Address;
use ethrex_l2_sdk::{get_last_committed_batch, get_last_verified_batch};
#[cfg(feature = "metrics")]
use ethrex_metrics::{
    l2::metrics::{METRICS, MetricsBlockType, MetricsOperationType},
    transactions::METRICS_TX,
};
use ethrex_rpc::clients::eth::EthClient;
use reqwest::Url;
use serde::Serialize;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response, send_after},
};
use std::{collections::BTreeMap, time::Duration};
use tracing::{debug, error};

#[protocol]
pub trait MetricsGathererProtocol: Send + Sync {
    fn gather(&self) -> Result<(), ActorError>;
    fn health(&self) -> Response<MetricsGathererHealth>;
}

pub struct MetricsGatherer {
    l1_eth_client: EthClient,
    l2_eth_client: EthClient,
    on_chain_proposer_address: Address,
    check_interval: Duration,
    rollup_store: StoreRollup,
}

#[derive(Clone, Serialize)]
pub struct MetricsGathererHealth {
    pub l1_rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    pub l2_rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    pub on_chain_proposer_address: Address,
    pub check_interval: Duration,
}

#[actor(protocol = MetricsGathererProtocol)]
impl MetricsGatherer {
    pub fn new(
        rollup_store: StoreRollup,
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
        l2_url: Url,
    ) -> Result<Self, MetricsGathererError> {
        let l1_eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let l2_eth_client = EthClient::new(l2_url)?;
        Ok(Self {
            l1_eth_client,
            l2_eth_client,
            rollup_store,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            check_interval: Duration::from_millis(5000),
        })
    }

    pub async fn spawn(
        cfg: &SequencerConfig,
        rollup_store: StoreRollup,
        l2_url: Url,
    ) -> Result<ActorRef<MetricsGatherer>, MetricsGathererError> {
        let metrics = Self::new(rollup_store, &cfg.l1_committer, &cfg.eth, l2_url)?;
        Ok(metrics.start())
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        let _ = ctx
            .send(metrics_gatherer_protocol::Gather)
            .inspect_err(|e| error!("Failed to send initial Gather: {e}"));
    }

    #[send_handler]
    async fn handle_gather(
        &mut self,
        _msg: metrics_gatherer_protocol::Gather,
        ctx: &Context<Self>,
    ) {
        let _ = self
            .gather_metrics()
            .await
            .inspect_err(|err| error!("Metrics Gatherer Error: {}", err));
        send_after(
            self.check_interval,
            ctx.clone(),
            metrics_gatherer_protocol::Gather,
        );
    }

    #[request_handler]
    async fn handle_health(
        &mut self,
        _msg: metrics_gatherer_protocol::Health,
        _ctx: &Context<Self>,
    ) -> MetricsGathererHealth {
        let l1_rpc_healthcheck = self.l1_eth_client.test_urls().await;
        let l2_rpc_healthcheck = self.l2_eth_client.test_urls().await;

        MetricsGathererHealth {
            l1_rpc_healthcheck,
            l2_rpc_healthcheck,
            on_chain_proposer_address: self.on_chain_proposer_address,
            check_interval: self.check_interval,
        }
    }

    async fn gather_metrics(&self) -> Result<(), MetricsGathererError> {
        let last_committed_batch =
            get_last_committed_batch(&self.l1_eth_client, self.on_chain_proposer_address).await?;

        let last_verified_batch =
            get_last_verified_batch(&self.l1_eth_client, self.on_chain_proposer_address).await?;

        let l1_gas_price = self.l1_eth_client.get_gas_price().await?;
        let l2_gas_price = self.l2_eth_client.get_gas_price().await?;

        if let Ok(Some(last_verified_batch_blocks)) = self
            .rollup_store
            .get_block_numbers_by_batch(last_verified_batch)
            .await
            && let Some(last_block) = last_verified_batch_blocks.last()
        {
            METRICS.set_block_type_and_block_number(
                MetricsBlockType::LastVerifiedBlock,
                *last_block,
            )?;
        }

        if let Ok(operations_metrics) = self.rollup_store.get_operations_count().await {
            let (transactions, privileged_transactions, messages) = (
                operations_metrics[0],
                operations_metrics[1],
                operations_metrics[2],
            );
            METRICS.set_operation_by_type(
                MetricsOperationType::PrivilegedTransactions,
                privileged_transactions,
            )?;
            METRICS.set_operation_by_type(MetricsOperationType::L1Messages, messages)?;
            METRICS_TX.set_tx_count(transactions)?;
        }

        METRICS.set_block_type_and_block_number(
            MetricsBlockType::LastCommittedBatch,
            last_committed_batch,
        )?;
        METRICS.set_block_type_and_block_number(
            MetricsBlockType::LastVerifiedBatch,
            last_verified_batch,
        )?;
        METRICS.set_l1_gas_price(
            l1_gas_price
                .try_into()
                .map_err(|e: &str| MetricsGathererError::TryInto(e.to_string()))?,
        );
        METRICS.set_l2_gas_price(
            l2_gas_price
                .try_into()
                .map_err(|e: &str| MetricsGathererError::TryInto(e.to_string()))?,
        );

        debug!("L2 Metrics Gathered");
        Ok(())
    }
}
