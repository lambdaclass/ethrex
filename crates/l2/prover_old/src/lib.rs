pub mod backends;
pub mod prover;

pub mod config;
use config::ProverConfig;
use tracing::warn;

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}

#[cfg(feature = "risc0")]
pub use backends::risc0::*;

#[cfg(feature = "sp1")]
pub use backends::sp1::*;

#[cfg(not(any(feature = "risc0", feature = "sp1")))]
pub use backends::exec::*;
