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

    /// Core execution - runs the L1 stateless validator directly.
    ///
    /// `ProgramInput` is the spec's `statelessInputBytes`, so this is the same
    /// entrypoint the released guest ELF runs. `successful_validation = false`
    /// surfaces as `Err` so result-only callers (ef_tests) treat it as failure.
    #[cfg(not(feature = "l2"))]
    fn execute_core(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
        use libssz::SszDecode;

        let crypto = Arc::new(NativeCrypto);
        let output_bytes = ethrex_guest_program::l1::run_stateless_guest(&input, crypto);
        let output = ProgramOutput::from_ssz_bytes(&output_bytes)
            .map_err(|e| BackendError::execution(format!("output decode: {e:?}")))?;
        if !output.successful_validation {
            return Err(BackendError::execution(
                "stateless validation returned successful_validation = false",
            ));
        }
        Ok(output)
    }

    /// Core execution - runs the L2 batch guest directly.
    #[cfg(feature = "l2")]
    fn execute_core(input: ProgramInput) -> Result<ProgramOutput, BackendError> {
        let crypto = Arc::new(NativeCrypto);
        ethrex_guest_program::execution::execution_program(input, crypto)
            .map_err(BackendError::execution)
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
        // The old `ProgramInput::Direct` guard is gone with the variant: every L1
        // input is now real `statelessInputBytes`, so the zero-root sentinel it
        // protected against cannot arise.
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
