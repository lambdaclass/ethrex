use crate::{CommitterConfig, EthConfig, SequencerConfig, sequencer::errors::MetricsGathererError};
use ::ethrex_storage_rollup::StoreRollup;
use ethereum_types::Address;
use ethrex_l2_sdk::{get_last_committed_batch, get_last_verified_batch};
#[cfg(feature = "metrics")]
use ethrex_metrics::{
    l2::metrics::{METRICS, MetricsBlockType, MetricsOperationType},
    metrics_transactions::METRICS_TX,
};
use ethrex_rpc::clients::eth::EthClient;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_after,
};
use std::time::Duration;
use tracing::{debug, error};

#[derive(Clone)]
pub enum InMessage {
    Gather,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct MetricsGatherer {
    l1_eth_client: EthClient,
    l2_eth_client: EthClient,
    on_chain_proposer_address: Address,
    check_interval: Duration,
    rollup_store: StoreRollup,
}

impl MetricsGatherer {
    pub async fn new(
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
            check_interval: Duration::from_millis(5000),
        })
    }

    pub async fn spawn(
        cfg: &SequencerConfig,
        rollup_store: StoreRollup,
        l2_url: String,
    ) -> Result<(), MetricsGathererError> {
        let mut metrics = Self::new(rollup_store, &(cfg.l1_committer.clone()), &cfg.eth, l2_url)
            .await?
            .start();
        metrics
            .cast(InMessage::Gather)
            .await
            .map_err(MetricsGathererError::InternalError)
    }

    async fn gather_metrics(&mut self) -> Result<(), MetricsGathererError> {
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
        {
            if let Some(last_block) = last_verified_batch_blocks.last() {
                METRICS.set_block_type_and_block_number(
                    MetricsBlockType::LastVerifiedBlock,
                    *last_block,
                )?;
            }
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

impl GenServer for MetricsGatherer {
    type CallMsg = ();
    type CastMsg = InMessage;
    type OutMsg = OutMessage;

    type Error = MetricsGathererError;

    async fn handle_call(
        &mut self,
        _message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        CallResponse::Reply(OutMessage::Done)
    }

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        // Right now we only have the Gather message, so we ignore the message
        let _ = self
            .gather_metrics()
            .await
            .inspect_err(|err| error!("Metrics Gatherer Error: {}", err));
        send_after(self.check_interval, handle.clone(), Self::CastMsg::Gather);
        CastResponse::NoReply
    }
}
