pub mod backend;
pub mod prover;

pub mod config;
use config::ProverConfig;
use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use guest_program::input::ProgramInput;
use tracing::warn;

use crate::backend::{Backend, ProveOutput};

#[derive(Debug)]
pub struct BackendNotAvailable(Backend);

impl std::fmt::Display for BackendNotAvailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Backend '{:?}' was not compiled. Enable the corresponding feature to use this backend.", self.0)
    }
}

impl std::error::Error for BackendNotAvailable {}

pub async fn init_client(config: ProverConfig) {
    prover::start_prover(config).await;
    warn!("Prover finished!");
}

/// Execute a program using the specified backend.
pub fn execute(backend: Backend, input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    match backend {
        Backend::Exec => backend::exec::execute(input),
        Backend::SP1 => {
            #[cfg(feature = "sp1")]
            {
                backend::sp1::execute(input)
            }
            #[cfg(not(feature = "sp1"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::SP1)))
            }
        },
        Backend::RISC0 => {
            #[cfg(feature = "risc0")]
            {
                backend::risc0::execute(input)
            }
            #[cfg(not(feature = "risc0"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::RISC0)))
            }
        },
        Backend::ZisK => {
            #[cfg(feature = "zisk")]
            {
                backend::zisk::execute(input)
            }
            #[cfg(not(feature = "zisk"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::ZisK)))
            }
        },
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
        Backend::SP1 => {
            #[cfg(feature = "sp1")]
            {
                backend::sp1::prove(input, format).map(ProveOutput::SP1)
            }
            #[cfg(not(feature = "sp1"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::SP1)))
            }
        },
        Backend::RISC0 => {
            #[cfg(feature = "risc0")]
            {
                backend::risc0::prove(input, format).map(ProveOutput::RISC0)
            }
            #[cfg(not(feature = "risc0"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::RISC0)))
            }
        },
        Backend::ZisK => {
            #[cfg(feature = "zisk")]
            {
                backend::zisk::prove(input, format).map(ProveOutput::ZisK)
            }
            #[cfg(not(feature = "zisk"))]
            {
                Err(Box::new(BackendNotAvailable(Backend::ZisK)))
            }
        },
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
    }
}
