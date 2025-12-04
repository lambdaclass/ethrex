pub mod backend;
pub mod prover;

pub mod config;
use config::ProverConfig;
use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::input::ProgramInput;
use std::time::Duration;
use tracing::warn;

use crate::backend::{Backend, ProveOutput};

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}

/// Execute a program using the specified backend.
pub fn execute(backend: Backend, input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backend::exec::execute(input),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backend::sp1::execute(input),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backend::risc0::execute(input),
        #[cfg(feature = "zisk")]
        Backend::ZisK => backend::zisk::execute(input),
        #[cfg(feature = "openvm")]
        Backend::OpenVM => backend::openvm::execute(input),
    }
}

/// Execute a program using the specified backend and measure the duration.
pub fn execute_timed(
    backend: Backend,
    input: ProgramInput,
) -> Result<Duration, Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backend::exec::execute_timed(input),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backend::sp1::execute_timed(input),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backend::risc0::execute_timed(input),
        #[cfg(feature = "zisk")]
        Backend::ZisK => backend::zisk::execute_timed(input),
        #[cfg(feature = "openvm")]
        Backend::OpenVM => backend::openvm::execute_timed(input),
    }
}

/// Generate a proof using the specified backend.
pub fn prove(
    backend: Backend,
    input: ProgramInput,
    format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backend::exec::prove(input, format).map(ProveOutput::Exec),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backend::sp1::prove(input, format).map(ProveOutput::SP1),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backend::risc0::prove(input, format).map(ProveOutput::RISC0),
        #[cfg(feature = "zisk")]
        Backend::ZisK => backend::zisk::prove(input, format).map(ProveOutput::ZisK),
        #[cfg(feature = "openvm")]
        Backend::OpenVM => backend::openvm::prove(input, format).map(ProveOutput::OpenVM),
    }
}

/// Generate a proof using the specified backend and measure the duration.
pub fn prove_timed(
    backend: Backend,
    input: ProgramInput,
    format: ProofFormat,
) -> Result<(ProveOutput, Duration), Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backend::exec::prove_timed(input, format)
            .map(|(output, duration)| (ProveOutput::Exec(output), duration)),
        #[cfg(feature = "sp1")]
        Backend::SP1 => backend::sp1::prove_timed(input, format)
            .map(|(output, duration)| (ProveOutput::SP1(output), duration)),
        #[cfg(feature = "risc0")]
        Backend::RISC0 => backend::risc0::prove_timed(input, format)
            .map(|(receipt, duration)| (ProveOutput::RISC0(receipt), duration)),
        #[cfg(feature = "zisk")]
        Backend::ZisK => backend::zisk::prove_timed(input, format)
            .map(|(proof, duration)| (ProveOutput::ZisK(proof), duration)),
        #[cfg(feature = "openvm")]
        Backend::OpenVM => backend::openvm::prove_timed(input, format)
            .map(|(proof, duration)| (ProveOutput::OpenVM(proof), duration)),
    }
}

pub fn to_batch_proof(
    proof: ProveOutput,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    match proof {
        ProveOutput::Exec(proof) => backend::exec::to_batch_proof(proof, format),
        #[cfg(feature = "sp1")]
        ProveOutput::SP1(proof) => backend::sp1::to_batch_proof(proof, format),
        #[cfg(feature = "risc0")]
        ProveOutput::RISC0(receipt) => backend::risc0::to_batch_proof(receipt, format),
        #[cfg(feature = "zisk")]
        ProveOutput::ZisK(proof) => backend::zisk::to_batch_proof(proof, format),
        #[cfg(feature = "openvm")]
        ProveOutput::OpenVM(proof) => backend::openvm::to_batch_proof(proof, format),
    }
}
