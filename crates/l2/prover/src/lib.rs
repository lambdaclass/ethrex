pub mod backend;
pub mod config;
pub mod prover;

use config::ProverConfig;
use tracing::warn;

pub use crate::backend::{BackendError, BackendType, ExecBackend, ProverBackend};

#[cfg(feature = "sp1")]
pub use crate::backend::Sp1Backend;

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}
