use crate::{
    backend::{BackendType, ExecBackend},
    config::ProverConfig,
};
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2::sequencer::utils::get_git_commit_hash;
use ethrex_l2_common::prover::ProverInputData;
use ethrex_prover_backend::prover::{InputConverter, Prover, ProverLoopConfig};

/// L2-specific converter from ProverInputData to ProgramInput.
struct L2InputConverter;

impl InputConverter for L2InputConverter {
    fn convert(&self, input: ProverInputData) -> ProgramInput {
        #[cfg(feature = "l2")]
        {
            ProgramInput {
                blocks: input.blocks,
                execution_witness: input.execution_witness,
                elasticity_multiplier: input.elasticity_multiplier,
                blob_commitment: input.blob_commitment,
                blob_proof: input.blob_proof,
                fee_configs: input.fee_configs,
            }
        }
        #[cfg(not(feature = "l2"))]
        {
            ProgramInput {
                blocks: input.blocks,
                execution_witness: input.execution_witness,
            }
        }
    }
}

fn make_config(cfg: &ProverConfig) -> ProverLoopConfig {
    ProverLoopConfig {
        proof_coordinator_endpoints: cfg.proof_coordinators.clone(),
        proving_time_ms: cfg.proving_time_ms,
        timed: cfg.timed,
        commit_hash: get_git_commit_hash(),
    }
}

pub async fn start_prover(config: ProverConfig) {
    match config.backend {
        BackendType::Exec => {
            let loop_config = make_config(&config);
            let prover = Prover::new(ExecBackend::new(), L2InputConverter, loop_config);
            prover.start().await;
        }
        #[cfg(feature = "sp1")]
        BackendType::SP1 => {
            use crate::backend::sp1::{PROVER_SETUP, Sp1Backend, init_prover_setup};
            #[cfg(feature = "gpu")]
            PROVER_SETUP.get_or_init(|| init_prover_setup(config.sp1_server.clone()));
            #[cfg(not(feature = "gpu"))]
            PROVER_SETUP.get_or_init(|| init_prover_setup(None));
            let loop_config = make_config(&config);
            let prover = Prover::new(Sp1Backend::new(), L2InputConverter, loop_config);
            prover.start().await;
        }
        #[cfg(feature = "risc0")]
        BackendType::RISC0 => {
            use crate::backend::Risc0Backend;
            let loop_config = make_config(&config);
            let prover = Prover::new(Risc0Backend::new(), L2InputConverter, loop_config);
            prover.start().await;
        }
        #[cfg(feature = "zisk")]
        BackendType::ZisK => {
            use crate::backend::ZiskBackend;
            let loop_config = make_config(&config);
            let prover = Prover::new(ZiskBackend::new(), L2InputConverter, loop_config);
            prover.start().await;
        }
        #[cfg(feature = "openvm")]
        BackendType::OpenVM => {
            use crate::backend::OpenVmBackend;
            let loop_config = make_config(&config);
            let prover = Prover::new(OpenVmBackend::new(), L2InputConverter, loop_config);
            prover.start().await;
        }
    }
}
