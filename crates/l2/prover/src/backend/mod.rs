use ethrex_l2_common::prover::{BatchProof, ProofCalldata, ProverType};

use crate::guest_program::input::ProgramInput;

pub mod exec;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(feature = "sp1")]
pub mod sp1;

// Note: Methods have &self as a parameter to make this trait dyn compatible, and
// for future backends that may require state.
pub trait ProverBackend {
    type ProgramOutput;
    type Error;

    fn r#type(&self) -> ProverType;

    fn execute(&self, input: ProgramInput) -> Result<(), Self::Error>;

    fn prove(
        &self,
        input: ProgramInput,
        aligned_mode: bool,
    ) -> Result<Self::ProgramOutput, Self::Error>;

    fn verify(&self, output: Self::ProgramOutput) -> Result<(), Self::Error>;

    fn to_batch_proof(
        &self,
        output: Self::ProgramOutput,
        aligned_mode: bool,
    ) -> Result<BatchProof, Self::Error>;

    fn to_calldata(&self, output: Self::ProgramOutput) -> Result<ProofCalldata, Self::Error>;
}
