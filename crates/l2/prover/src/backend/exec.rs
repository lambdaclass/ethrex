use tracing::{info, warn};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProofFormat, ProverType},
};
use guest_program::{input::ProgramInput, output::ProgramOutput};

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

    fn execution_program(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
        guest_program::execution::execution_program(input).map_err(BackendError::execution)
    }

    fn to_calldata(proof: &ProgramOutput) -> ProofCalldata {
        let public_inputs = proof.encode();
        ProofCalldata {
            prover_type: ProverType::Exec,
            calldata: vec![Value::Bytes(public_inputs.into())],
        }
    }
}

impl ProverBackend for ExecBackend {
    type ProofOutput = ProgramOutput;

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let now = std::time::Instant::now();
        Self::execution_program(input)?;
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
        Self::execution_program(input)
    }

    fn verify(&self, _proof: &Self::ProofOutput) -> Result<(), BackendError> {
        warn!("\"exec\" prover backend generates no proof, verification always succeeds");
        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        Ok(BatchProof::ProofCalldata(Self::to_calldata(&proof)))
    }
}
