use ethrex_common::types::TxType;
use metrics::{counter, gauge};
use prometheus::{
    Encoder, Gauge, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_TX: LazyLock<MetricsTx> = LazyLock::new(MetricsTx::default);

#[derive(Debug, Clone)]
pub struct MetricsTx {
    pub transactions_tracker: IntCounterVec,
    pub transaction_errors_count: IntCounterVec,
    pub transactions_total: IntGauge,
    pub mempool_tx_count: IntGaugeVec,
    pub transactions_per_second: Gauge,
}

impl Default for MetricsTx {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsTx {
    pub fn new() -> Self {
        let transactions_tracker = IntCounterVec::new(
            Opts::new(
                "old_transactions_tracker",
                "[DEPRECATED] Keeps track of all transactions depending on status and tx_type",
            ),
            &["tx_type"],
        )
        .unwrap();

        Self::initialize_transactions_tracker(&transactions_tracker);

        MetricsTx {
            transactions_tracker,
            transaction_errors_count: IntCounterVec::new(
                Opts::new(
                    "old_transaction_errors_count",
                    "[DEPRECATED] Keeps track of all errors that happen during transaction execution",
                ),
                &["tx_error"],
            )
            .unwrap(),
            transactions_total: IntGauge::new(
                "old_transactions_total",
                "[DEPRECATED] Keeps track of all transactions",
            )
            .unwrap(),
            mempool_tx_count: IntGaugeVec::new(
                Opts::new(
                    "old_mempool_tx_count",
                    "[DEPRECATED] Keeps track of the amount of txs on the mempool",
                ),
                &["type"],
            )
            .unwrap(),
            transactions_per_second: Gauge::new(
                "old_transactions_per_second",
                "[DEPRECATED] Keeps track of the TPS",
            )
            .unwrap(),
        }
    }

    pub fn inc_tx_with_type(&self, tx_type: MetricsTxType) {
        let txs = self.transactions_tracker.clone();

        let txs_builder = match txs.get_metric_with_label_values(&[tx_type.to_str()]) {
            Ok(builder) => builder,
            Err(e) => {
                tracing::error!("Failed to build Metric: {e}");
                return;
            }
        };

        txs_builder.inc();
        // Record to new metrics system
        counter!(
            "transactions_total",
            "tx_type" => tx_type.to_str().to_string()
        )
        .increment(1);
    }

    pub fn inc_tx_errors(&self, tx_error: &str) {
        let tx_errors = self.transaction_errors_count.clone();

        let tx_errors_builder = match tx_errors.get_metric_with_label_values(&[tx_error]) {
            Ok(builder) => builder,
            Err(e) => {
                tracing::error!("Failed to build Metric: {e}");
                return;
            }
        };

        tx_errors_builder.inc();
        // Record to new metrics system
        counter!(
            "transaction_errors_total",
            "error_type" => tx_error.to_string()
        )
        .increment(1);
    }

    pub fn set_tx_count(&self, count: u64) -> Result<(), MetricsError> {
        self.transactions_total.set(count.try_into()?);
        gauge!("transactions_count").set(count as f64);
        Ok(())
    }

    pub fn set_mempool_tx_count(&self, count: u64, is_blob: bool) -> Result<(), MetricsError> {
        let label = if is_blob { "blob" } else { "regular" };

        let builder = self
            .mempool_tx_count
            .get_metric_with_label_values(&[label])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        builder.set(count.try_into()?);
        gauge!("mempool_tx_count", "type" => label.to_string()).set(count as f64);

        Ok(())
    }

    pub fn set_transactions_per_second(&self, tps: f64) {
        self.transactions_per_second.set(tps);
        gauge!("transactions_per_second").set(tps);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.transactions_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.transactions_tracker.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.transaction_errors_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.mempool_tx_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.transactions_per_second.clone()))
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

    fn initialize_transactions_tracker(transactions_tracker: &IntCounterVec) {
        for tx_type in MetricsTxType::all() {
            transactions_tracker.with_label_values(&[&tx_type]).reset();
        }
    }
}

pub struct MetricsTxType(pub TxType);

impl MetricsTxType {
    pub fn to_str(&self) -> &str {
        match self.0 {
            ethrex_common::types::TxType::Legacy => "Legacy",
            ethrex_common::types::TxType::EIP2930 => "EIP2930",
            ethrex_common::types::TxType::EIP1559 => "EIP1559",
            ethrex_common::types::TxType::EIP4844 => "EIP4844",
            ethrex_common::types::TxType::EIP7702 => "EIP7702",
            ethrex_common::types::TxType::Privileged => "Privileged",
            ethrex_common::types::TxType::FeeToken => "FeeTokenTransaction",
        }
    }
    pub fn all() -> Vec<String> {
        vec![
            "Legacy".to_string(),
            "EIP2930".to_string(),
            "EIP1559".to_string(),
            "EIP4844".to_string(),
            "EIP7702".to_string(),
            "Privileged".to_string(),
            "FeeToken".to_string(),
        ]
    }
}
