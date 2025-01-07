use prometheus::{Encoder, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::{Arc, LazyLock, Mutex};

use crate::MetricsError;

pub static METRICS_L2: LazyLock<MetricsL2> = LazyLock::new(MetricsL2::default);

pub struct MetricsL2 {
    pub status_tracker: Arc<Mutex<IntGaugeVec>>,
}

impl Default for MetricsL2 {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsL2 {
    pub fn new() -> Self {
        MetricsL2 {
            status_tracker: Arc::new(Mutex::new(
                IntGaugeVec::new(
                    Opts::new(
                        "l2_blocks_tracker",
                        "Keeps track of the L2's status based on the L1's contracts",
                    ),
                    &["block_type"],
                )
                .unwrap(),
            )),
        }
    }

    pub fn set_block_type_and_block_number(
        &self,
        block_type: MetricsL2BlockType,
        block_number: u64,
    ) -> Result<(), MetricsError> {
        let clone = self.status_tracker.clone();

        let lock = clone
            .lock()
            .map_err(|e| MetricsError::MutexLockError(e.to_string()))?;

        let builder = lock
            .get_metric_with_label_values(&[block_type.to_str()])
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        let block_number_as_i64: i64 = block_number.try_into()?;

        builder.set(block_number_as_i64);

        Ok(())
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        let clone = self.status_tracker.clone();

        let lock = clone
            .lock()
            .map_err(|e| MetricsError::MutexLockError(e.to_string()))?;

        r.register(Box::new(lock.clone()))
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

/// [MetricsL2BlockType::LastCommittedBlock] and [MetricsL2BlockType::LastVerifiedBlock] Matche the crates/l2/contracts/src/l1/OnChainProposer.sol variables
/// [MetricsL2BlockType::LastFetchedL1Block] Matches the variable in crates/l2/contracts/src/l1/CommonBridge.sol
pub enum MetricsL2BlockType {
    LastCommittedBlock,
    LastVerifiedBlock,
    LastFetchedL1Block,
}

impl MetricsL2BlockType {
    pub fn to_str(&self) -> &str {
        match self {
            MetricsL2BlockType::LastCommittedBlock => "lastCommittedBlock",
            MetricsL2BlockType::LastVerifiedBlock => "lastVerifiedBlock",
            MetricsL2BlockType::LastFetchedL1Block => "lastFetchedL1Block",
        }
    }
}
