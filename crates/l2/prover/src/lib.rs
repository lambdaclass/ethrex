pub mod errors;
pub mod prover;
pub mod prover_client;

use ethrex_l2::{
    proposer::prover_server::ProverType, utils::config::prover_client::ProverClientConfig,
};
use tracing::warn;

pub async fn init_client(config: ProverClientConfig, prover_type: ProverType) {
    prover_client::start_proof_data_client(config, prover_type).await;
    warn!("Prover finished!");
}
