use crate::config::ProverConfig;
use ethrex_l2::sequencer::utils::get_git_commit_hash;
use ethrex_l2_common::prover::ProverInputData;
use ethrex_prover::prover::ProverPullConfig;

#[cfg(all(feature = "sp1", feature = "gpu"))]
use ethrex_prover::backend::sp1::{PROVER_SETUP, init_prover_setup};

pub async fn start_prover(config: ProverConfig) {
    #[cfg(all(feature = "sp1", feature = "gpu"))]
    if config.backend == ethrex_prover::BackendType::SP1 {
        PROVER_SETUP.get_or_init(|| init_prover_setup(config.sp1_server.clone()));
    }

    let pull_config = ProverPullConfig {
        proof_coordinator_endpoints: config.proof_coordinators,
        proving_time_ms: config.proving_time_ms,
        timed: config.timed,
        commit_hash: get_git_commit_hash(),
    };

    ethrex_prover::prover::start_prover::<ProverInputData>(config.backend, pull_config).await;
}
