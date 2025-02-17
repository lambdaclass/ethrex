use crate::proposer::errors::ProverServerError;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[cfg(feature = "risc0")]
use risc0_zkvm::sha::Digestible;
use sp1_sdk::{ExecutionReport as SP1ExecutionReport, HashableKey, SP1PublicValues};

/// Enum used to identify the different proving systems.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProverType {
    RISC0,
    SP1,
    Pico,
}

/// Used to iterate through all the possible proving systems
impl ProverType {
    pub fn all() -> &'static [ProverType] {
        &[roverType::SP1]
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ProvingOutput {
    #[cfg(feature = "risc0")]
    RISC0(Risc0Proof),
    #[cfg(feature = "sp1")]
    SP1(Sp1Proof),
    #[cfg(feature = "pico")]
    Pico(PicoProof),
}

impl From<ProvingOutput> for ProverType {
    fn from(value: ProvingOutput) -> Self {
        match value {
            #[cfg(feature = "risc0")]
            ProvingOutput::RISC0(_) => ProverType::RISC0,
            #[cfg(feature = "sp1")]
            ProvingOutput::SP1(_) => ProverType::SP1,
            ProvingOutput::Pico(_) => ProverType::Pico,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ExecuteOutput {
    // TODO: Risc0
    // TODO: Pico
    SP1((SP1PublicValues, SP1ExecutionReport)),
}
