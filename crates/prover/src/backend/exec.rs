use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::crypto::NativeCrypto;
use ethrex_guest_program::{input::ProgramInput, output::ProgramOutput};

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
        let crypto = Arc::new(NativeCrypto);
        #[cfg(feature = "eip-8025")]
        {
            let output = ethrex_guest_program::l1::execute_decoded(input, crypto)
                .map_err(BackendError::execution)?;
            // Surface canonical/legacy `valid = false` as an Err so callers that
            // only inspect the Result (e.g. ef_tests' `(expected_valid, exec_result)`
            // match) treat it as execution failure, matching the legacy-path
            // semantics where invalid blocks bubble out as ExecutionError.
            if !output.valid {
                return Err(BackendError::execution(
                    "eip-8025 stateless execution: valid=false",
                ));
            }
            Ok(output)
        }
        #[cfg(not(feature = "eip-8025"))]
        {
            ethrex_guest_program::execution::execution_program(input, crypto)
                .map_err(BackendError::execution)
        }
    }

    fn empty_proof_bytes() -> ProverOutput {
        // Use a non-empty sentinel so that the proof pipeline accepts this
        // output (engine_verifyExecutionProofV1 rejects empty proof_data).
        ProverOutput::Proof(ProofBytes {
            prover_type: ProverType::Exec,
            proof: vec![0x00],
        })
    }
}

impl ProverBackend for ExecBackend {
    type ProofOutput = ProgramOutput;
    type SerializedInput = ();

    fn prover_type(&self) -> ProverType {
        ProverType::Exec
    }

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

    fn to_proof_bytes(
        &self,
        _proof: Self::ProofOutput,
        _format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        Ok(Self::empty_proof_bytes())
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let start = Instant::now();
        Self::execute_core(input)?;
        let elapsed = start.elapsed();
        info!("Successfully executed program in {:.2?}", elapsed);
        Ok(elapsed)
    }
}
