use prometheus::{Encoder, Gauge, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_BLOCKS: LazyLock<MetricsBlocks> = LazyLock::new(MetricsBlocks::default);

#[derive(Debug, Clone)]
pub struct MetricsBlocks {
    gas_limit: Gauge,
    block_number: IntGauge,
    gigagas: Gauge,
    gas_used: Gauge,
}

impl Default for MetricsBlocks {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsBlocks {
    pub fn new() -> Self {
        MetricsBlocks {
            gas_limit: Gauge::new(
                "gas_limit",
                "Keeps track of the percentage of gas limit used by the last block",
            )
            .unwrap(),
            block_number: IntGauge::new(
                "block_number",
                "Keeps track of the block number for the head of the chain",
            )
            .unwrap(),
            gigagas: Gauge::new(
                "gigagas",
                "Keeps track of the block building throughput through gigagas/s",
            )
            .unwrap(),
            gas_used: Gauge::new(
                "gas_used",
                "Keeps track of the gas used in the latest block",
            )
            .unwrap(),
        }
    }

    pub fn set_latest_block_gas_limit(&self, gas_limit: f64) {
        self.gas_limit.set(gas_limit);
    }

    pub fn set_latest_gigagas(&self, gigagas: f64) {
        self.gigagas.set(gigagas);
    }

    pub fn set_block_number(&self, block_number: u64) -> Result<(), MetricsError> {
        self.block_number.set(block_number.try_into()?);
        Ok(())
    }

    pub fn set_latest_gas_used(&self, gas_used: f64) {
        self.gas_used.set(gas_used);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.gas_limit.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_number.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gas_used.clone()))
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
