use std::time::{Duration, Instant};

use tracing::{info, warn};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProofFormat, ProverType},
};
use crate::zkvm::{ProgramInput, ProgramOutput, execution_program};

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

    fn run_execution_program(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
        execution_program(input).map_err(BackendError::execution)
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
        let now = std::time::Instant::now();
        Self::run_execution_program(input)?;
        let elapsed = now.elapsed();

        info!("Successfully executed program in {:.2?}", elapsed);
        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        warn!("\"exec\" prover backend generates no proof, only executes");
        Self::run_execution_program(input)
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
        Self::run_execution_program(input)?;
        let elapsed = start.elapsed();
        info!("Successfully executed program in {:.2?}", elapsed);
        Ok(elapsed)
    }
}
