pub mod config;
pub mod prover;

// Re-export the backend module from the shared crate
pub use ethrex_prover_core::backend;

use config::ProverConfig;
use tracing::warn;

pub use ethrex_prover_core::{BackendError, BackendType, ExecBackend, ProverBackend};

#[cfg(feature = "sp1")]
pub use ethrex_prover_core::Sp1Backend;

#[cfg(feature = "risc0")]
pub use ethrex_prover_core::Risc0Backend;

#[cfg(feature = "zisk")]
pub use ethrex_prover_core::ZiskBackend;

#[cfg(feature = "openvm")]
pub use ethrex_prover_core::OpenVmBackend;

// Re-export protocol and prover types from shared crate
pub use ethrex_prover_core::protocol;

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}
