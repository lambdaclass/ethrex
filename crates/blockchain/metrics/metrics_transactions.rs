use ethrex_core::types::TxType;
use prometheus::{Encoder, IntCounter, IntCounterVec, Opts, Registry, TextEncoder};
use std::sync::{Arc, LazyLock, Mutex};

use crate::MetricsError;

pub static METRICS_TX: LazyLock<MetricsTx> = LazyLock::new(MetricsTx::default);

pub struct MetricsTx {
    pub transactions_tracker: Arc<Mutex<IntCounterVec>>,
    pub transactions_total: Arc<Mutex<IntCounter>>,
}

impl Default for MetricsTx {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsTx {
    pub fn new() -> Self {
        MetricsTx {
            transactions_tracker: Arc::new(Mutex::new(
                IntCounterVec::new(
                    Opts::new(
                        "transactions_tracker",
                        "Keeps track of all transactions depending on status and tx_type",
                    ),
                    &["status", "tx_type"],
                )
                .unwrap(),
            )),
            transactions_total: Arc::new(Mutex::new(
                IntCounter::new("transactions_total", "Keeps track of all transactions").unwrap(),
            )),
        }
    }

    pub fn inc_tx_with_status_and_type(&self, status: MetricsTxStatus, tx_type: MetricsTxType) {
        let txs = self.transactions_tracker.clone();

        let txs_lock = match txs.lock() {
            Ok(lock) => lock,
            Err(e) => {
                tracing::error!("Failed to lock mutex: {e}");
                return;
            }
        };

        let txs_builder =
            match txs_lock.get_metric_with_label_values(&[status.to_str(), tx_type.to_str()]) {
                Ok(builder) => builder,
                Err(e) => {
                    tracing::error!("Failed to build Metric: {e}");
                    return;
                }
            };

        txs_builder.inc();
    }

    pub fn inc_tx(&self) {
        let txs = self.transactions_total.clone();

        let txs_lock = match txs.lock() {
            Ok(lock) => lock,
            Err(e) => {
                tracing::error!("Failed to lock mutex: {e}");
                return;
            }
        };

        txs_lock.inc();
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        let txs_tracker = self.transactions_tracker.clone();
        let txs_tracker_lock = txs_tracker
            .lock()
            .map_err(|e| MetricsError::MutexLockError(e.to_string()))?;

        let txs_lock = self.transactions_total.clone();
        let txs_lock = txs_lock
            .lock()
            .map_err(|e| MetricsError::MutexLockError(e.to_string()))?;

        r.register(Box::new(txs_lock.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(txs_tracker_lock.clone()))
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

pub enum MetricsTxStatus {
    Failed,
    Succeeded,
}

impl MetricsTxStatus {
    pub fn to_str(&self) -> &str {
        match self {
            MetricsTxStatus::Failed => "failed",
            MetricsTxStatus::Succeeded => "succedded",
        }
    }
}

pub struct MetricsTxType(pub TxType);

impl MetricsTxType {
    pub fn to_str(&self) -> &str {
        match self.0 {
            ethrex_core::types::TxType::Legacy => "Legacy",
            ethrex_core::types::TxType::EIP2930 => "EIP2930",
            ethrex_core::types::TxType::EIP1559 => "EIP1559",
            ethrex_core::types::TxType::EIP4844 => "EIP4844",
            ethrex_core::types::TxType::Privileged => "Privileged",
        }
    }
}
