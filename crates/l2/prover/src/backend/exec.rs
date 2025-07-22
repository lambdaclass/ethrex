use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProverType},
};
use tracing::warn;

use crate::{
    backend::ProverBackend,
    guest_program::{execution::execution_program, input::ProgramInput, output::ProgramOutput},
};

#[derive(Debug, thiserror::Error)]
pub enum ExecBackendError {
    #[error("Failed to execute program: {0}")]
    ProgramExecutionError(String),
}

pub struct ExecBackend;

impl ProverBackend for ExecBackend {
    type ProgramOutput = ProgramOutput;

    type Error = ExecBackendError;

    fn r#type(&self) -> ProverType {
        ProverType::Exec
    }

    fn execute(&self, input: ProgramInput) -> Result<(), Self::Error> {
        execution_program(input)
            .map_err(|err| Self::Error::ProgramExecutionError(err.to_string()))?;

        Ok(())
    }

    fn prove(
        &self,
        input: ProgramInput,
        _aligned_mode: bool,
    ) -> Result<Self::ProgramOutput, Self::Error> {
        warn!("\"exec\" prover backend generates no proof, only executes");

        execution_program(input).map_err(|err| Self::Error::ProgramExecutionError(err.to_string()))
    }

    fn verify(&self, _proof: Self::ProgramOutput) -> Result<(), Self::Error> {
        warn!("\"exec\" prover backend generates no proof, verification always succeeds");

        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProgramOutput,
        _aligned_mode: bool,
    ) -> Result<ethrex_l2_common::prover::BatchProof, Self::Error> {
        let batch_proof = BatchProof::ProofCalldata(self.to_calldata(proof)?);

        Ok(batch_proof)
    }

    fn to_calldata(&self, proof: Self::ProgramOutput) -> Result<ProofCalldata, Self::Error> {
        let calldata = ProofCalldata {
            prover_type: self.r#type(),
            calldata: vec![Value::Bytes(proof.encode().into())],
        };

        Ok(calldata)
    }
}
