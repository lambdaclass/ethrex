use prometheus::{Encoder, Gauge, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::default);

pub struct Metrics {
    status_tracker: IntGaugeVec,
    operations_tracker: IntGaugeVec,
    l1_gas_price: IntGauge,
    l2_gas_price: IntGauge,
    blob_usage: Gauge,
    batch_size: IntGaugeVec,
    batch_gas_used: IntGaugeVec,
    batch_proving_time: IntGaugeVec,
    batch_verification_gas: IntGaugeVec,
    batch_commitment_gas: IntGaugeVec,
    batch_commitment_blob_gas: IntGaugeVec,
    batch_tx_count: IntGaugeVec,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            status_tracker: IntGaugeVec::new(
                Opts::new(
                    "l2_blocks_tracker",
                    "Keeps track of the L2's status based on the L1's contracts",
                ),
                &["block_type"],
            )
            .unwrap(),
            operations_tracker: IntGaugeVec::new(
                Opts::new(
                    "l2_operations_tracker",
                    "Keeps track of the L2 deposits & withdrawals",
                ),
                &["operations_type"],
            )
            .unwrap(),
            l1_gas_price: IntGauge::new("l1_gas_price", "Keeps track of the l1 gas price").unwrap(),
            l2_gas_price: IntGauge::new("l2_gas_price", "Keeps track of the l2 gas price").unwrap(),
            blob_usage: Gauge::new(
                "l2_blob_usage",
                "Keeps track of the percentage of blob usage for a batch commitment",
            )
            .unwrap(),
            batch_size: IntGaugeVec::new(
                Opts::new(
                    "batch_size",
                    "Batch size in blocks, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
            batch_gas_used: IntGaugeVec::new(
                Opts::new(
                    "batch_gas_used",
                    "Batch total gas used, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
            batch_proving_time: IntGaugeVec::new(
                Opts::new(
                    "batch_proving_time",
                    "Time it took to prove a batch in seconds, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
            batch_verification_gas: IntGaugeVec::new(
                Opts::new(
                    "batch_verification_gas",
                    "Batch verification gas cost in L1, labeled by batch number and tx hash",
                ),
                &["batch_number", "tx_hash"],
            )
            .unwrap(),
            batch_commitment_gas: IntGaugeVec::new(
                Opts::new(
                    "batch_commitment_gas",
                    "Batch commitment gas cost in L1, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
            batch_commitment_blob_gas: IntGaugeVec::new(
                Opts::new(
                    "batch_commitment_blob_gas",
                    "Batch commitment blob gas cost in L1, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
            batch_tx_count: IntGaugeVec::new(
                Opts::new(
                    "batch_tx_count",
                    "Batch transaction count, labeled by batch number",
                ),
                &["batch_number"],
            )
            .unwrap(),
        }
    }

    pub fn set_l1_gas_price(&self, gas_price: i64) {
        self.l1_gas_price.set(gas_price);
    }

    pub fn set_l2_gas_price(&self, gas_price: i64) {
        self.l2_gas_price.set(gas_price);
    }

    pub fn set_block_type_and_block_number(
        &self,
        block_type: MetricsBlockType,
        block_number: u64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .status_tracker
            .get_metric_with_label_values(&[block_type.to_str()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        let block_number_as_i64: i64 = block_number.try_into()?;

        builder.set(block_number_as_i64);

        Ok(())
    }

    pub fn set_operation_by_type(
        &self,
        operation_type: MetricsOperationType,
        amount: u64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .operations_tracker
            .get_metric_with_label_values(&[operation_type.to_str()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        builder.set(amount.try_into()?);

        Ok(())
    }

    pub fn set_blob_usage_percentage(&self, usage: f64) {
        self.blob_usage.set(usage);
    }

    pub fn set_batch_size(&self, batch_number: u64, size: i64) -> Result<(), MetricsError> {
        let builder = self
            .batch_size
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(size);
        Ok(())
    }

    pub fn set_batch_gas_used(
        &self,
        batch_number: u64,
        total_gas: i64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .batch_gas_used
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(total_gas);
        Ok(())
    }

    pub fn set_batch_proving_time(
        &self,
        batch_number: u64,
        proving_time: i64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .batch_proving_time
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(proving_time);
        Ok(())
    }

    pub fn set_batch_verification_gas(
        &self,
        batch_number: u64,
        verification_gas: i64,
        tx_hash: &str,
    ) -> Result<(), MetricsError> {
        let builder = self
            .batch_verification_gas
            .get_metric_with_label_values(&[&batch_number.to_string(), tx_hash])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(verification_gas);
        Ok(())
    }

    pub fn set_batch_commitment_gas(
        &self,
        batch_number: u64,
        commitment_gas: i64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .batch_commitment_gas
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(commitment_gas);
        Ok(())
    }

    pub fn set_batch_commitment_blob_gas(
        &self,
        batch_number: u64,
        commitment_blob_gas: i64,
    ) -> Result<(), MetricsError> {
        let builder = self
            .batch_commitment_blob_gas
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(commitment_blob_gas);
        Ok(())
    }

    pub fn set_batch_tx_count(&self, batch_number: u64, tx_count: i64) -> Result<(), MetricsError> {
        let builder = self
            .batch_tx_count
            .get_metric_with_label_values(&[&batch_number.to_string()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        builder.set(tx_count);
        Ok(())
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.status_tracker.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.l1_gas_price.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.l2_gas_price.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.operations_tracker.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.blob_usage.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_size.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_gas_used.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_proving_time.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_verification_gas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_commitment_gas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_commitment_blob_gas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.batch_tx_count.clone()))
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

/// [MetricsBlockType::LastCommittedBatch] and [MetricsBlockType::LastVerifiedBatch] Matche the crates/l2/contracts/src/l1/OnChainProposer.sol variables
pub enum MetricsBlockType {
    LastCommittedBlock,
    LastVerifiedBlock,
    LastCommittedBatch,
    LastVerifiedBatch,
}

pub enum MetricsOperationType {
    PrivilegedTransactions,
    L1Messages,
}

impl MetricsBlockType {
    pub fn to_str(&self) -> &str {
        match self {
            MetricsBlockType::LastCommittedBlock => "lastCommittedBlock",
            MetricsBlockType::LastVerifiedBlock => "lastVerifiedBlock",
            MetricsBlockType::LastCommittedBatch => "lastCommittedBatch",
            MetricsBlockType::LastVerifiedBatch => "lastVerifiedBatch",
        }
    }
}

impl MetricsOperationType {
    fn to_str(&self) -> &str {
        match self {
            MetricsOperationType::PrivilegedTransactions => "processedPrivilegedTransactions",
            MetricsOperationType::L1Messages => "processedMessages",
        }
    }
}
