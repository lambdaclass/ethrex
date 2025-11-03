use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_SYNC: LazyLock<MetricsSync> = LazyLock::new(MetricsSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSync {
    /// Number of active peers connected
    active_peers: IntGauge,
    /// Best peer block number
    best_peer_block: IntGauge,
    /// Current head block number
    head_block: IntGauge,
    /// Difference between best peer and head
    peer_lag_blocks: IntGauge,
    /// Finalized block number
    finalized_block: IntGauge,
    /// Safe block number
    safe_block: IntGauge,
}

impl Default for MetricsSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSync {
    pub fn new() -> Self {
        MetricsSync {
            active_peers: IntGauge::new(
                "active_peers",
                "Number of active peers connected to the node",
            )
            .unwrap(),
            best_peer_block: IntGauge::new(
                "best_peer_block",
                "Block number of the best peer in the network",
            )
            .unwrap(),
            head_block: IntGauge::new("head_block", "Current head block number of the node")
                .unwrap(),
            peer_lag_blocks: IntGauge::new(
                "peer_lag_blocks",
                "Number of blocks the node is lagging behind the best peer",
            )
            .unwrap(),
            finalized_block: IntGauge::new("finalized_block", "Latest finalized block number")
                .unwrap(),
            safe_block: IntGauge::new("safe_block", "Latest safe block number").unwrap(),
        }
    }

    pub fn set_active_peers(&self, count: u64) -> Result<(), MetricsError> {
        self.active_peers.set(count.try_into()?);
        Ok(())
    }

    pub fn set_best_peer_block(&self, block_number: u64) -> Result<(), MetricsError> {
        self.best_peer_block.set(block_number.try_into()?);
        Ok(())
    }

    pub fn set_head_block(&self, block_number: u64) -> Result<(), MetricsError> {
        self.head_block.set(block_number.try_into()?);
        Ok(())
    }

    pub fn set_peer_lag_blocks(&self, lag: u64) -> Result<(), MetricsError> {
        self.peer_lag_blocks.set(lag.try_into()?);
        Ok(())
    }

    pub fn set_finalized_block(&self, block_number: u64) -> Result<(), MetricsError> {
        self.finalized_block.set(block_number.try_into()?);
        Ok(())
    }

    pub fn set_safe_block(&self, block_number: u64) -> Result<(), MetricsError> {
        self.safe_block.set(block_number.try_into()?);
        Ok(())
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.active_peers.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.best_peer_block.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.head_block.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.peer_lag_blocks.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.finalized_block.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.safe_block.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let encoder = TextEncoder::new();
        let metric_families = r.gather();

        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let res = String::from_utf8(buffer)?;

        Ok(res)
    }
}
