use ethrex_common::types::NETWORK_NAMES;
use prometheus::{Encoder, GaugeVec, IntGaugeVec, Opts, Registry, TextEncoder};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use crate::MetricsError;

pub static METRICS_BLOCKS: LazyLock<MetricsBlocks> = LazyLock::new(MetricsBlocks::default);

#[derive(Debug)]
pub struct MetricsBlocks {
    gas_limit: GaugeVec,
    gigagas: GaugeVec,
    gigagas_block_building: GaugeVec,
    block_building_ms: IntGaugeVec,
    block_building_base_fee: IntGaugeVec,
    gas_used: GaugeVec,
    /// Keeps track of the head block number
    head_height: IntGaugeVec,
    active_block_labels: Mutex<HashMap<(String, String), String>>,
}

impl Default for MetricsBlocks {
    fn default() -> Self {
        Self::new()
    }
}

const BLOCK_LABELS: &[&str] = &["network", "chain_id", "block_number"];
const NETWORK_LABELS: &[&str] = &["network", "chain_id"];

#[derive(Debug, Clone)]
pub struct NetworkLabels {
    network: Cow<'static, str>,
    chain_id: String,
}

impl NetworkLabels {
    pub fn new(chain_id: u64) -> Self {
        let network = NETWORK_NAMES
            .get(&chain_id)
            .copied()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("chain-{chain_id}")));

        Self {
            network,
            chain_id: chain_id.to_string(),
        }
    }

    pub fn network_name(&self) -> &str {
        self.network.as_ref()
    }

    fn values(&self) -> [&str; 2] {
        [self.network.as_ref(), &self.chain_id]
    }
}

#[derive(Debug, Clone)]
pub struct BlockMetricLabels {
    network: NetworkLabels,
    block_number: String,
}

impl BlockMetricLabels {
    pub fn new(chain_id: u64, block_number: u64) -> Self {
        Self {
            network: NetworkLabels::new(chain_id),
            block_number: block_number.to_string(),
        }
    }

    fn values(&self) -> [&str; 3] {
        [
            self.network.network.as_ref(),
            &self.network.chain_id,
            &self.block_number,
        ]
    }

    pub fn network_labels(&self) -> &NetworkLabels {
        &self.network
    }
}

impl MetricsBlocks {
    pub fn new() -> Self {
        MetricsBlocks {
            gas_limit: GaugeVec::new(
                Opts::new(
                    "gas_limit",
                    "Keeps track of the percentage of gas limit used by the last processed block",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            gigagas: GaugeVec::new(
                Opts::new(
                    "gigagas",
                    "Keeps track of the block execution throughput through gigagas/s",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            gigagas_block_building: GaugeVec::new(
                Opts::new(
                    "gigagas_block_building",
                    "Keeps track of the block building throughput through gigagas/s",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            block_building_ms: IntGaugeVec::new(
                Opts::new(
                    "block_building_ms",
                    "Keeps track of the block building throughput through miliseconds",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            block_building_base_fee: IntGaugeVec::new(
                Opts::new(
                    "block_building_base_fee",
                    "Keeps track of the block building base fee",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            gas_used: GaugeVec::new(
                Opts::new(
                    "gas_used",
                    "Keeps track of the gas used in the last processed block",
                ),
                BLOCK_LABELS,
            )
            .unwrap(),
            head_height: IntGaugeVec::new(
                Opts::new(
                    "head_height",
                    "Keeps track of the block number for the head of the chain",
                ),
                NETWORK_LABELS,
            )
            .unwrap(),
            active_block_labels: Mutex::new(HashMap::new()),
        }
    }

    fn recycle_previous_block_labels(
        &self,
        labels: &BlockMetricLabels,
    ) -> Result<(), MetricsError> {
        let mut active = self
            .active_block_labels
            .lock()
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let key = (
            labels.network.network_name().to_string(),
            labels.network.chain_id.clone(),
        );

        if active
            .get(&key)
            .is_some_and(|prev_block| prev_block == &labels.block_number)
        {
            return Ok(());
        }

        let previous_block = active.insert(key, labels.block_number.clone());
        drop(active);

        if let Some(previous_block) = previous_block {
            let old_values = [
                labels.network.network_name(),
                labels.network.chain_id.as_str(),
                previous_block.as_str(),
            ];

            let remove_err = |e: prometheus::Error| MetricsError::PrometheusErr(e.to_string());

            self.gas_limit
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
            self.gigagas
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
            self.gigagas_block_building
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
            self.block_building_ms
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
            self.block_building_base_fee
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
            self.gas_used
                .remove_label_values(&old_values)
                .map_err(remove_err)?;
        }

        Ok(())
    }

    pub fn set_latest_block_gas_limit(
        &self,
        labels: &BlockMetricLabels,
        gas_limit: f64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.gas_limit
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(gas_limit);
        Ok(())
    }

    pub fn set_latest_gigagas(
        &self,
        labels: &BlockMetricLabels,
        gigagas: f64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.gigagas
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(gigagas);
        Ok(())
    }

    pub fn set_latest_gigagas_block_building(
        &self,
        labels: &BlockMetricLabels,
        gigagas: f64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.gigagas_block_building
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(gigagas);
        Ok(())
    }

    pub fn set_block_building_ms(
        &self,
        labels: &BlockMetricLabels,
        ms: i64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.block_building_ms
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(ms);
        Ok(())
    }

    pub fn set_block_building_base_fee(
        &self,
        labels: &BlockMetricLabels,
        base_fee: i64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.block_building_base_fee
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(base_fee);
        Ok(())
    }

    pub fn set_head_height(
        &self,
        labels: &NetworkLabels,
        head_height: u64,
    ) -> Result<(), MetricsError> {
        let values = labels.values();
        self.head_height
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(head_height.try_into()?);
        Ok(())
    }

    pub fn set_latest_gas_used(
        &self,
        labels: &BlockMetricLabels,
        gas_used: f64,
    ) -> Result<(), MetricsError> {
        self.recycle_previous_block_labels(labels)?;
        let values = labels.values();
        self.gas_used
            .get_metric_with_label_values(&values)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?
            .set(gas_used);
        Ok(())
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.gas_limit.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas_block_building.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gas_used.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_building_base_fee.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_building_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.head_height.clone()))
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
