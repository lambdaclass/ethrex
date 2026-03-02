use std::time::{Duration, Instant};

use tracing::{info, warn};

use ethrex_guest_program::{input::ProgramInput, output::ProgramOutput, traits::backends};
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

    fn prover_type(&self) -> ProverType {
        ProverType::Exec
    }

    fn backend_name(&self) -> &'static str {
        backends::EXEC
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

    fn execute_with_elf(&self, _elf: &[u8], serialized_input: &[u8]) -> Result<(), BackendError> {
        // Exec mode ignores the ELF and runs execution_program directly.
        // Deserialize the rkyv bytes back to ProgramInput.
        let input: ProgramInput =
            rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>(serialized_input)
                .map_err(|e| BackendError::serialization(e.to_string()))?;
        Self::execute_core(input)?;
        Ok(())
    }

    fn prove_with_elf(
        &self,
        _elf: &[u8],
        serialized_input: &[u8],
        _format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        warn!("\"exec\" prover backend generates no proof, only executes (ELF path)");
        let input: ProgramInput =
            rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>(serialized_input)
                .map_err(|e| BackendError::serialization(e.to_string()))?;
        Self::execute_core(input)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::backend::ProverBackend;

    #[test]
    fn backend_name_is_exec() {
        let backend = ExecBackend::new();
        assert_eq!(backend.backend_name(), "exec");
    }

    #[test]
    fn execute_with_elf_invalid_input_returns_serialization_error() {
        let backend = ExecBackend::new();
        let bad_input = b"not valid rkyv bytes";
        let result = backend.execute_with_elf(&[], bad_input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, BackendError::Serialization(_)),
            "expected Serialization error, got: {err:?}"
        );
    }

    #[test]
    fn prove_with_elf_invalid_input_returns_serialization_error() {
        let backend = ExecBackend::new();
        let bad_input = b"not valid rkyv bytes";
        let result = backend.prove_with_elf(&[], bad_input, ProofFormat::Compressed);
        match result {
            Err(BackendError::Serialization(_)) => {} // expected
            Err(other) => panic!("expected Serialization error, got: {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn execute_with_elf_empty_input_returns_error() {
        let backend = ExecBackend::new();
        let result = backend.execute_with_elf(&[], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn prove_with_elf_empty_input_returns_error() {
        let backend = ExecBackend::new();
        let result = backend.prove_with_elf(&[], &[], ProofFormat::Compressed);
        assert!(matches!(result, Err(_)));
    }

    #[test]
    fn serialize_raw_produces_deserializable_bytes() {
        use ethrex_common::types::block_execution_witness::ExecutionWitness;

        let backend = ExecBackend::new();
        let input = ProgramInput {
            blocks: vec![],
            execution_witness: ExecutionWitness::default(),
            elasticity_multiplier: 0,
            fee_configs: vec![],
            blob_commitment: [0u8; 48],
            blob_proof: [0u8; 48],
        };

        // serialize_raw should produce valid rkyv bytes.
        let bytes = backend
            .serialize_raw(&input)
            .expect("serialize_raw should succeed");
        assert!(!bytes.is_empty(), "serialized bytes should not be empty");

        // The bytes should be deserializable back to ProgramInput.
        let roundtripped: ProgramInput =
            rkyv::from_bytes::<ProgramInput, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialization should succeed");
        assert_eq!(roundtripped.blocks.len(), 0);
        assert_eq!(roundtripped.elasticity_multiplier, 0);
    }
}
