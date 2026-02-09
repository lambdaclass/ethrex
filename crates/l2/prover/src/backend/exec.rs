use std::time::{Duration, Instant};

use tracing::{info, warn};

use ethrex_guest_program::{input::ProgramInput, output::ProgramOutput};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProofFormat, ProverType},
};

use crate::backend::{BackendError, ProverBackend};

/// Exec backend - executes the program without generating actual proofs.
///
/// This backend is useful for testing and debugging, as it runs the guest
/// program directly without the overhead of proof generation.
#[derive(Default)]
pub struct ExecBackend;

impl ExecBackend {
    pub fn new() -> Self {
        Self
    }

    /// Core execution - runs the guest program directly.
    fn execute_core(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
        ethrex_guest_program::execution::execution_program(input).map_err(BackendError::execution)
    }

    fn to_calldata() -> ProofCalldata {
        ProofCalldata {
            prover_type: ProverType::Exec,
            calldata: vec![Value::Bytes(vec![].into())],
        }
    }
}

impl ProverBackend for ExecBackend {
    type ProofOutput = ProgramOutput;
    type SerializedInput = ();

    fn serialize_input(
        &self,
        _input: &ProgramInput,
    ) -> Result<Self::SerializedInput, BackendError> {
        // ExecBackend doesn't serialize - it passes input directly to execution_program
        Ok(())
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        Self::execute_core(input)?;
        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        warn!("\"exec\" prover backend generates no proof, only executes");
        Self::execute_core(input)
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        warn!("\"exec\" prover backend generates no proof, verification always succeeds");
        Ok(())
    }

    fn to_batch_proof(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        Ok(BatchProof::ProofCalldata(Self::to_calldata()))
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let start = Instant::now();
        Self::execute_core(input)?;
        let elapsed = start.elapsed();
        info!("Successfully executed program in {:.2?}", elapsed);
        Ok(elapsed)
    }
}
