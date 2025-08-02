pub mod backends;
pub mod prover;

pub mod config;
use config::ProverConfig;
use ethrex_l2_common::prover::BatchProof;
use tracing::warn;
use zkvm_interface::io::ProgramInput;

use crate::backends::{Backend, ProveOutput};

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}

pub fn execute(backend: Backend, input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backends::exec::execute(input),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backends::sp1::execute(input),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backends::risc0::execute(input),
    }
}

pub fn prove(
    backend: Backend,
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backends::exec::prove(input, aligned_mode).map(ProveOutput::Exec),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backends::sp1::prove(input, aligned_mode).map(ProveOutput::SP1),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backends::risc0::prove(input, aligned_mode).map(ProveOutput::RISC0),
    }
}

pub fn to_batch_proof(
    proof: ProveOutput,
    aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    match proof {
        ProveOutput::Exec(proof) => backends::exec::to_batch_proof(proof, aligned_mode),
        #[cfg(feature = "sp1")]
        ProveOutput::SP1(proof) => backends::sp1::to_batch_proof(proof, aligned_mode),
        #[cfg(feature = "risc0")]
        ProveOutput::RISC0(receipt) => backends::risc0::to_batch_proof(receipt, aligned_mode),
    }
}
