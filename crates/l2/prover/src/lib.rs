pub mod prover;
pub mod prover_client;

use ethrex_l2::utils::config::prover_client::ProverClientConfig;
use tracing::warn;

pub async fn init_client(config: ProverClientConfig) {
    prover_client::start_proof_data_client(config).await;
    warn!("Prover finished!");
}
