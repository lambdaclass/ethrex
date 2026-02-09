use std::str::FromStr;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2_common::prover::{BatchProof, ProofFormat, ProverType};
use serde::{Deserialize, Serialize};

pub mod error;
pub mod exec;

#[cfg(feature = "risc0")]
pub mod risc0;

#[cfg(feature = "sp1")]
pub mod sp1;

#[cfg(feature = "zisk")]
pub mod zisk;

#[cfg(feature = "openvm")]
pub mod openvm;

pub use error::BackendError;

// Re-export backend structs
pub use exec::ExecBackend;

#[cfg(feature = "risc0")]
pub use risc0::Risc0Backend;

#[cfg(feature = "sp1")]
pub use sp1::Sp1Backend;

#[cfg(feature = "zisk")]
pub use zisk::ZiskBackend;

#[cfg(feature = "openvm")]
pub use openvm::OpenVmBackend;

/// Enum for selecting which backend to use (for CLI/config).
#[derive(Default, Debug, Deserialize, Serialize, Copy, Clone, ValueEnum, PartialEq)]
pub enum BackendType {
    #[default]
    Exec,
    #[cfg(feature = "sp1")]
    SP1,
    #[cfg(feature = "risc0")]
    RISC0,
    #[cfg(feature = "zisk")]
    ZisK,
    #[cfg(feature = "openvm")]
    OpenVM,
}

// Needed for Clap
impl FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "exec" => Ok(BackendType::Exec),
            #[cfg(feature = "sp1")]
            "sp1" => Ok(BackendType::SP1),
            #[cfg(feature = "risc0")]
            "risc0" => Ok(BackendType::RISC0),
            #[cfg(feature = "zisk")]
            "zisk" => Ok(BackendType::ZisK),
            #[cfg(feature = "openvm")]
            "openvm" => Ok(BackendType::OpenVM),
            _ => Err(Self::Err::from("Invalid backend")),
        }
    }
}

/// Trait defining the interface for prover backends.
///
/// All proving backends (SP1, RISC0, ZisK, OpenVM, Exec) implement this trait,
/// providing a unified interface for execution, proving, verification, and
/// batch proof conversion.
pub trait ProverBackend {
    /// The proof output type specific to this backend.
    type ProofOutput;

    /// The serialized input type specific to this backend.
    type SerializedInput;

    /// Returns the prover type this backend produces.
    fn prover_type(&self) -> ProverType;

    /// Serialize the program input into the backend-specific format.
    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError>;

    /// Serialize the program input and measure the duration.
    ///
    /// Default implementation wraps `serialize_input` with timing.
    fn serialize_input_timed(
        &self,
        input: &ProgramInput,
    ) -> Result<(Self::SerializedInput, Duration), BackendError> {
        let start = Instant::now();
        let serialized = self.serialize_input(input)?;
        Ok((serialized, start.elapsed()))
    }

    /// Execute the program without generating a proof (for testing/debugging).
    fn execute(&self, input: ProgramInput) -> Result<(), BackendError>;

    /// Generate a proof for the given input.
    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError>;

    /// Verify a proof.
    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError>;

    /// Convert backend-specific proof to unified BatchProof format.
    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<BatchProof, BackendError>;

    /// Execute the program and measure the duration.
    ///
    /// Default implementation wraps `execute` with timing.
    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let start = Instant::now();
        self.execute(input)?;
        Ok(start.elapsed())
    }

    /// Generate a proof and measure the duration.
    ///
    /// Default implementation wraps `prove` with timing.
    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let start = Instant::now();
        let proof = self.prove(input, format)?;
        Ok((proof, start.elapsed()))
    }
}
