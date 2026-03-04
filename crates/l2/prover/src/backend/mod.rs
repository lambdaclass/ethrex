use std::str::FromStr;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use ethrex_guest_program::input::ProgramInput;
use ethrex_guest_program::traits::backends;
use ethrex_l2_common::prover::{BatchProof, ProofFormat, ProverType};
use rkyv::rancor::Error as RkyvError;
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

impl BackendType {
    /// Returns the backend name string matching
    /// [`ethrex_guest_program::traits::backends`] constants.
    ///
    /// This is used to ask a [`GuestProgram`] for the correct ELF binary.
    pub fn as_backend_name(&self) -> &'static str {
        match self {
            BackendType::Exec => backends::EXEC,
            #[cfg(feature = "sp1")]
            BackendType::SP1 => backends::SP1,
            #[cfg(feature = "risc0")]
            BackendType::RISC0 => backends::RISC0,
            #[cfg(feature = "zisk")]
            BackendType::ZisK => backends::ZISK,
            #[cfg(feature = "openvm")]
            BackendType::OpenVM => backends::OPENVM,
        }
    }
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
///
/// # ELF-based methods
///
/// The `*_with_elf` methods accept an explicit ELF binary and pre-serialized
/// input bytes instead of relying on compiled-in constants and the concrete
/// [`ProgramInput`] type.  This decouples the backend from the guest program,
/// allowing different guest programs to be proven by the same backend.
///
/// All `*_with_elf` methods have default implementations that return
/// [`BackendError::NotImplemented`] so that existing backends continue to
/// compile while they are migrated incrementally.
pub trait ProverBackend {
    /// The proof output type specific to this backend.
    type ProofOutput;

    /// The serialized input type specific to this backend.
    type SerializedInput;

    /// Returns the ProverType for this backend.
    fn prover_type(&self) -> ProverType;

    /// Returns the backend name as a string matching the well-known constants
    /// in [`ethrex_guest_program::traits::backends`].
    ///
    /// This is used to ask a [`GuestProgram`](ethrex_guest_program::traits::GuestProgram)
    /// for the correct ELF binary.
    fn backend_name(&self) -> &'static str;

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

    /// Serialize the program input into raw bytes (rkyv format).
    ///
    /// Returns the backend-agnostic byte representation of `ProgramInput`.
    /// These bytes are then passed through `GuestProgram::serialize_input()`
    /// and fed to `*_with_elf` methods.
    ///
    /// The default implementation uses rkyv.  Backends can override if they
    /// need a different wire format.
    fn serialize_raw(&self, input: &ProgramInput) -> Result<Vec<u8>, BackendError> {
        let bytes = rkyv::to_bytes::<RkyvError>(input).map_err(BackendError::serialization)?;
        Ok(bytes.to_vec())
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

    // -- ELF-based methods (guest-program agnostic) --------------------------

    /// Execute a guest program given its ELF binary and pre-serialized input.
    ///
    /// `serialized_input` contains the bytes the guest program reads from the
    /// zkVM stdin (typically rkyv-encoded `ProgramInput`).
    fn execute_with_elf(&self, _elf: &[u8], _serialized_input: &[u8]) -> Result<(), BackendError> {
        Err(BackendError::not_implemented("execute_with_elf"))
    }

    /// Generate a proof given an explicit ELF binary and pre-serialized input.
    fn prove_with_elf(
        &self,
        _elf: &[u8],
        _serialized_input: &[u8],
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        Err(BackendError::not_implemented("prove_with_elf"))
    }

    /// Execute with an explicit ELF and measure the duration.
    fn execute_with_elf_timed(
        &self,
        elf: &[u8],
        serialized_input: &[u8],
    ) -> Result<Duration, BackendError> {
        let start = Instant::now();
        self.execute_with_elf(elf, serialized_input)?;
        Ok(start.elapsed())
    }

    /// Prove with an explicit ELF and measure the duration.
    fn prove_with_elf_timed(
        &self,
        elf: &[u8],
        serialized_input: &[u8],
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let start = Instant::now();
        let proof = self.prove_with_elf(elf, serialized_input, format)?;
        Ok((proof, start.elapsed()))
    }
}
